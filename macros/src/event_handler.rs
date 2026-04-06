use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::{
    Attribute, Field, FieldMutability, Fields, FieldsNamed, FnArg, Ident, ImplItem, ImplItemFn,
    ItemEnum, ItemImpl, Pat, PatType, Variant, parse::Parse, parse_quote, parse_quote_spanned,
    spanned::Spanned as _,
};

#[derive(Default, Debug)]
pub struct HandlerArgs {
    event_envelope: Option<Ident>,
    event_ident: Option<Ident>,
    inherit_expr: Option<syn::Expr>,
}
impl HandlerArgs {
    fn get_envelope(&self) -> Ident {
        match self.event_envelope.clone() {
            Some(ident) => {
                let mut ident = ident.clone();
                ident.set_span(Span::call_site());
                ident
            }
            None => Ident::new("EventEnvelope", Span::call_site()),
        }
    }
    fn get_event(&self) -> Ident {
        match self.event_ident.clone() {
            Some(ident) => {
                let mut ident = ident.clone();
                ident.set_span(Span::call_site());
                ident
            }
            None => Ident::new("Event", Span::call_site()),
        }
    }
}
impl Parse for HandlerArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut handler_args = HandlerArgs::default();
        let punctuated: syn::punctuated::Punctuated<syn::Meta, syn::token::Comma> =
            input.parse_terminated(syn::Meta::parse, syn::Token![,])?;
        let mut position = 0;
        for meta in punctuated {
            position += 1;
            match meta {
                syn::Meta::Path(path) if position == 1 => {
                    if let Some(ident) = path.get_ident() {
                        handler_args.event_ident = Some(ident.clone());
                    }
                }
                syn::Meta::Path(_) => (),
                syn::Meta::List(_) => (),
                syn::Meta::NameValue(meta_name_value)
                    if meta_name_value.path.is_ident("inherit") =>
                {
                    handler_args.inherit_expr = Some(meta_name_value.value);
                }
                syn::Meta::NameValue(meta_name_value)
                    if meta_name_value.path.is_ident("envelope") =>
                {
                    if let syn::Expr::Path(p) = meta_name_value.value
                        && let Some(ident) = p.path.get_ident()
                    {
                        handler_args.event_envelope = Some(ident.clone());
                    }
                }
                syn::Meta::NameValue(_) => (),
            }
        }
        Ok(handler_args)
    }
}

pub(crate) fn event_processor_impl(args: HandlerArgs, input: TokenStream) -> TokenStream {
    let mut item_impl: ItemImpl = syn::parse2(input).expect("Cannot parse input");
    let mut context = ProcessorContext::default();
    context.args = args;

    // Extract identifier from the arguments (the THIS_ID part in #[event_processor(THIS_ID)])
    // let type_ident: Ident = syn::parse2(arguments)
    //     .expect("Expected identifier in #[event_processor(...)]");
    // context.event_enum = type_ident;

    let mut i = 0;
    while i < item_impl.items.len() {
        if let ImplItem::Fn(ref mut method) = item_impl.items[i] {
            if let Some(attr) = find_and_remove_handler_attr(&mut method.attrs) {
                let handler = process_handler_method(&context.args, method, attr);
                context.push_handler(handler);
                item_impl.items.remove(i);
                continue;
            }
        }
        i += 1;
    }

    generate_output(item_impl, context)
}

#[derive(Default)]
struct ProcessorContext {
    event_preprocess: bool,
    match_arms: Vec<TokenStream>,
    debug_arms: Vec<TokenStream>,
    handlers: Vec<ImplItem>,
    events: Vec<Variant>,
    event_new_methods: Vec<ImplItemFn>,
    args: HandlerArgs,
    errors: Vec<TokenStream>,
}

impl ProcessorContext {
    fn push_handler(&mut self, handler: HandlerResult) {
        let variant_ident = &handler.variant_ident;
        let method_name = &handler.method_name;
        let event_match = if handler.has_fields {
            quote! {{..}}
        } else {
            quote! {}
        };

        let mut event_preprocess_error = TokenStream::new();
        if self.event_preprocess && handler.is_event_preprocess {
            event_preprocess_error = quote_spanned! { method_name.span() =>
                compile_error!("Only one event_preprocess handle can be used");
            };
        }
        self.event_preprocess = handler.is_event_preprocess;
        let ei = &self.args.get_event();
        self.match_arms.push(quote_spanned! { handler.span =>
            #ei::#variant_ident #event_match => self.#method_name(env, queue).await
        });
        self.errors.push(event_preprocess_error);

        self.debug_arms.push(quote! {
            #ei::#variant_ident #event_match => write!(f, stringify!(#variant_ident))
        });

        self.events.push(handler.event_variant);
        for method in handler.generated_methods {
            self.handlers.push(method);
        }

        self.event_new_methods.push(handler.event_new_method);
    }
}

struct HandlerResult {
    span: Span,
    variant_ident: Ident,
    method_name: Ident,
    event_variant: Variant,
    generated_methods: Vec<ImplItem>,
    has_fields: bool,
    is_event_preprocess: bool,
    event_new_method: syn::ImplItemFn,
}

fn process_handler_method(
    args: &HandlerArgs,
    method: &ImplItemFn,
    attr: Attribute,
) -> HandlerResult {
    let variant_ident: Ident = attr.parse_args().expect("Expected ident in #[handler()]");
    let span = method.sig.span();
    let method_name = method.sig.ident.clone();
    let method_body = &method.block;
    let mut method_attrs = method.attrs.clone();
    let is_impure = find_and_remove_impure_attr(&mut method_attrs).is_some();
    let is_event_preprocess = find_and_remove_event_preprocess_attr(&mut method_attrs).is_some();

    let (envelope_ident, queue_ident, other_args) = analyze_method_inputs(args, &method.sig.inputs);

    let span_assignment = envelope_ident
        .map(|id| quote! { let #id = env; })
        .unwrap_or_default();
    let queue_assignment = queue_ident
        .map(|id| quote! { let #id = queue; })
        .unwrap_or_default();

    let pat_fields: Vec<PatType> = other_args
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat) = arg {
                Some(pat.clone())
            } else {
                None
            }
        })
        .collect();

    let event_variant = create_event_variant(variant_ident.clone(), &pat_fields);
    let guard_stmt = create_guard_statement(&variant_ident, &args.get_event(), &pat_fields);
    let has_fields = !pat_fields.is_empty();
    let event_envelope_ident = &args.get_envelope();

    let event_new_method = create_event_method(
        variant_ident.clone(),
        &pat_fields,
        &args.get_event(),
        event_envelope_ident,
    );

    let method_actual: ImplItem = parse_quote! {
        #(#method_attrs),*
        async fn #method_name(&self, env: #event_envelope_ident, #[allow(unused)] queue: &mut ::std::collections::VecDeque<#event_envelope_ident>) {
            #guard_stmt
            self.event_preprocess(&env.event);
            #queue_assignment
            #span_assignment
            #method_body
        }

    };
    //let disable_in_test = if is_impure { quote!{#[cfg(not(test))]}} else { TokenStream::new() };
    let mut generated_methods = Vec::new();
    if is_impure {
        generated_methods.push(parse_quote_spanned! { method.span() =>
            #[cfg(not(test))]
            #method_actual
        });
        let span = method.span().span().span().span();
        generated_methods.push(parse_quote_spanned! { span =>
            #[cfg(test)]
            #(#method_attrs),*
            async fn #method_name(&self, _env: #event_envelope_ident, _queue: &mut ::std::collections::VecDeque<#event_envelope_ident>) {
                panic!("#[impure] functions are stripped in test environment");
            }
        });
    } else {
        generated_methods.push(method_actual);
    };

    HandlerResult {
        span,
        variant_ident,
        method_name,
        event_variant,
        generated_methods,
        event_new_method,
        has_fields,
        is_event_preprocess,
    }
}

fn create_event_method(
    variant: Ident,
    pat_fields: &[PatType],
    event_ident: &Ident,
    envelope_ident: &Ident,
) -> syn::ImplItemFn {
    let new_method_name = Ident::new(
        &format!("new_{}", heck::AsSnakeCase(variant.to_string())),
        variant.span(),
    );

    let mut args: Vec<FnArg> = Vec::new();
    pat_fields
        .iter()
        .map(|pf| syn::FnArg::Typed(pf.clone()))
        .for_each(|arg| args.push(arg));

    let fields = pat_fields
        .iter()
        .filter_map(|pt| {
            if let Pat::Ident(ident) = *pt.pat.clone() {
                Some(ident.ident.to_token_stream())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let ts: syn::ImplItemFn = parse_quote! {
        pub fn #new_method_name(#(#args),*) -> #envelope_ident {
             return #envelope_ident {
                 span: ::tracing::Span::current(),
                 event: #event_ident::#variant {
                     id: ::uuid::Uuid::new_v4(),
                     #(#fields),*
                 }
             }
        }
    };
    ts
}

fn analyze_method_inputs(
    args: &HandlerArgs,
    inputs: &syn::punctuated::Punctuated<FnArg, syn::token::Comma>,
) -> (Option<Ident>, Option<Ident>, Vec<FnArg>) {
    let mut envelope_ident = None;
    let mut queue_ident = None;
    let mut other_args = Vec::new();

    for arg in inputs {
        if let FnArg::Typed(pat_type) = arg {
            let ty_str = pat_type.ty.to_token_stream().to_string();
            let is_envelope = if let syn::Type::Path(p) = *pat_type.ty.clone()
                && let Some(last) = p.path.segments.last()
                && last.ident.to_string() == args.get_envelope().to_string()
            {
                true
            } else {
                false
            };
            let is_queue =
                ty_str.contains("VecDeque") && ty_str.contains(&args.get_envelope().to_string());

            if is_envelope {
                if let Pat::Ident(pat_ident) = &*pat_type.pat {
                    envelope_ident = Some(pat_ident.ident.clone());
                }
            } else if is_queue {
                if let Pat::Ident(pat_ident) = &*pat_type.pat {
                    queue_ident = Some(pat_ident.ident.clone());
                }
            } else {
                other_args.push(arg.clone());
            }
        } else {
            other_args.push(arg.clone());
        }
    }

    (envelope_ident, queue_ident, other_args)
}

fn add_extra_fields(vec: &mut Vec<Field>) {
    let id_field = parse_quote! {
        id: ::uuid::Uuid
    };
    vec.insert(0, id_field);
}
fn create_event_variant(ident: Ident, pat_fields: &[PatType]) -> Variant {
    let mut fields_vec: Vec<Field> = pat_fields
        .iter()
        .filter_map(|p| {
            if let Pat::Ident(pat_ident) = &*p.pat {
                Some(Field {
                    attrs: p.attrs.clone(),
                    vis: parse_quote!(),
                    mutability: FieldMutability::None,
                    ident: Some(pat_ident.ident.clone()),
                    colon_token: None,
                    ty: (*p.ty).clone(),
                })
            } else {
                None
            }
        })
        .collect();
    add_extra_fields(&mut fields_vec);

    let fields = if fields_vec.is_empty() {
        Fields::Unit
    } else {
        Fields::Named(FieldsNamed {
            brace_token: Default::default(),
            named: fields_vec.into_iter().collect(),
        })
    };

    Variant {
        attrs: Vec::new(),
        ident,
        fields,
        discriminant: None,
    }
}

fn create_guard_statement(
    variant_ident: &Ident,
    event_ident: &Ident,
    pat_fields: &[PatType],
) -> TokenStream {
    let field_names: Vec<_> = pat_fields
        .iter()
        .map(|pf| {
            if let Pat::Ident(ident) = &*pf.pat {
                quote! { #ident }
            } else {
                panic!("Expected identifier in function arguments")
            }
        })
        .collect();

    let match_pattern = if field_names.is_empty() {
        quote! {}
    } else {
        quote! {{ id, #(#field_names),* }}
    };

    quote! {
        let #event_ident::#variant_ident #match_pattern = env.event else { unreachable!(); };
    }
}

fn generate_output(mut item_impl: ItemImpl, context: ProcessorContext) -> TokenStream {
    let ProcessorContext {
        event_preprocess,
        match_arms,
        debug_arms,
        handlers,
        events,
        event_new_methods,
        args,
        errors,
    } = context;
    let ei = &args.get_event();
    let event_envelope = &args.get_envelope();

    let event_enum = ItemEnum {
        attrs: parse_quote! { #[derive(::serde::Serialize, ::serde::Deserialize, Clone)] },
        vis: parse_quote!(pub),
        enum_token: Default::default(),
        ident: args.get_event(),
        generics: Default::default(),
        brace_token: Default::default(),
        variants: events.into_iter().collect(),
    };

    let debug_impl = quote! {
        impl std::fmt::Debug for #ei {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    #(#debug_arms),*
                    ,_ => write!(f, "Unknown Event"),
                }
            }
        }
    };
    item_impl.items.extend(handlers);

    if !event_preprocess {
        let ei = args.get_event();
        item_impl.items.push(parse_quote! {
            fn event_preprocess(&self, _event: &#ei){}

        });
    }
    if let Some(inherit) = args.inherit_expr.clone() {
        item_impl.items.push(parse_quote! {
            #[tracing::instrument(skip_all,parent=&env.span)]
            pub async fn handle(&self, env: #event_envelope, queue: &mut ::std::collections::VecDeque<#event_envelope>) {
                use #ei::*;
                tracing::debug!("handle: {:?}", env.event);
                self.event_preprocess(&env.event);
                match &env.event {
                    #(#match_arms),*,
                    _ => (#inherit).handle(env, queue).await,
                };
            }

        });
    } else {
        item_impl.items.push(parse_quote! {
            #[tracing::instrument(skip_all,parent=&env.span)]
            pub async fn handle(&self, env: #event_envelope, queue: &mut ::std::collections::VecDeque<#event_envelope>) {
                use #ei::*;
                tracing::debug!("handle: {:?}", env.event);
                match env.event {
                    #(#match_arms),*
                };
            }
        });
    }

    item_impl.items.push(parse_quote! {
        pub async fn run(&self, mut rx: ::tokio::sync::mpsc::Receiver<#event_envelope>) {
            let mut queue: ::std::collections::VecDeque<#event_envelope> = ::std::collections::VecDeque::new();
            loop {
                match rx.recv().await {
                    Some(envelope) => queue.push_back(envelope),
                    None => {
                        tracing::error!("event channel closed unexpectedly");
                        break;
                    }
                }
                while let Ok(envelope) = rx.try_recv() {
                    queue.push_back(envelope);
                }
                while let Some(envelope) = queue.pop_front() {
                    self.handle(envelope, &mut queue).await;
                }
            }
        }
    });

    item_impl
        .attrs
        .retain(|attr| !attr.path().is_ident("event_processor"));

    let ei = args.get_event();
    let ev = args.get_envelope();
    let mut event_into_envelope = TokenStream::new();
    if args.inherit_expr.is_none() {
        event_into_envelope = quote! {
            impl From<#ei> for #ev {
               fn from(input: #ei) -> #ev {
                   #ev {
                       span: ::tracing::Span::current(),
                       event: input
                   }
               }
            }

        };
    };
    let event_enum_ts = if args.inherit_expr.is_some() {
        quote! {}
    } else {
        event_enum.to_token_stream()
    };
    let debug_impl_ts = if args.inherit_expr.is_some() {
        quote! {}
    } else {
        debug_impl.to_token_stream()
    };

    let ei = args.get_event();
    let event_envelope = if args.inherit_expr.is_some() {
        quote! {}
    } else {
        quote! {
            pub struct #event_envelope {
                pub event: #ei,
                pub span: ::tracing::Span,
            }
        }
    };

    let mut event_enum_impl = TokenStream::new();
    if args.inherit_expr.is_none() {
        let ei = args.get_event();
        event_enum_impl = {
            quote! {
                impl #ei {
                    #(#event_new_methods)*
                }
            }
        };
    }
    let out = quote! {
        #event_envelope
        #event_enum_ts
        #event_enum_impl
        #event_into_envelope
        #debug_impl_ts
        #(#errors)*
        #item_impl
    };
    out
}

fn find_and_remove_handler_attr(attrs: &mut Vec<Attribute>) -> Option<Attribute> {
    let index = attrs
        .iter()
        .position(|attr| attr.path().is_ident("handler"))?;
    Some(attrs.remove(index))
}
fn find_and_remove_impure_attr(attrs: &mut Vec<Attribute>) -> Option<Attribute> {
    let index = attrs
        .iter()
        .position(|attr| attr.path().is_ident("impure"))?;
    Some(attrs.remove(index))
}
fn find_and_remove_event_preprocess_attr(attrs: &mut Vec<Attribute>) -> Option<Attribute> {
    let index = attrs
        .iter()
        .position(|attr| attr.path().is_ident("event_preprocess"))?;
    Some(attrs.remove(index))
}

#[allow(unused)]
fn pretty_print(tokens: proc_macro2::TokenStream) -> String {
    // 1. Parse the tokens into a syntax tree (syn::File)
    // Note: This requires the tokens to be a valid Rust file (items only)
    let Ok(syntax_tree) = syn::parse2::<syn::File>(tokens.clone()) else {
        return tokens.to_string();
    };

    // 2. Format it
    prettyplease::unparse(&syntax_tree)
}
#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_1() {
        let attr = quote! { #[event_processor] };
        let input = quote! {
            #attr
            impl EventHandler {
                #[handler(Event1)]
                fn handler(#[serde(skip)] test: Vec<Vec<u32>>) {
                    return;
                }
            }
        };
        let output = event_processor_impl(HandlerArgs::default(), input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }
    #[test]
    fn test_processor_1() {
        let attr = quote! { #[event_processor] };
        let input = quote! {
            #attr
            impl EventHandler {
                #[handler(MyStruct)]
                fn test_processor_1(&self, item: String, value: u32) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(HandlerArgs::default(), input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }

    #[test]
    fn test_processor_2() {
        let attr = quote! { #[event_processor] };
        let input = quote! {
            #[event_processor(#attr)]
            impl EventHandler {
                #[handler(EventNoFields)]
                fn test_processor_2(&self) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(HandlerArgs::default(), input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }

    #[test]
    fn test_processor_3() {
        let attr = quote! { #[event_processor] };
        let input = quote! {
            #[event_processor(#attr)]
            impl EventHandler {
                #[handler(EventNoFieldsEnv)]
                fn test_processor_3(&self, env: EventEnvelope) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(HandlerArgs::default(), input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }

    #[test]
    fn test_processor_4() {
        let attr = quote! { #[event_processor] };
        let input = quote! {
            #attr
            impl EventHandler {
                #[handler(EventNoFieldsEnvArc)]
                fn test_processor_4(&self, #[serde(skip)] arc: Option<Arc<Something>>) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(HandlerArgs::default(), input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }
    #[test]
    fn test_processor_5() {
        let attr = quote! { CustomEvent };
        let input = quote! {
            #[event_processor(#attr)]
            impl EventHandler {
                #[handler(EventFields)]
                fn test_processor_5(&self, value: u32, #[serde(skip)] arc: Option<Arc<Something>>) {
                    println!("Hello world!");
                }
            }
        };
        let args: HandlerArgs = syn::parse2(attr).unwrap();
        let output = event_processor_impl(args, input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }
    #[test]
    fn test_processor_6() {
        let attr = quote! { CustomEvent,envelope = EEEEnvelope };
        let input = quote! {
            #[event_processor(#attr)]
            impl EventHandler {
                #[handler(EventFields)]
                fn test_processor_6(&self, value: u32) {
                    println!("Hello world!");
                }
            }
        };
        let args: HandlerArgs = syn::parse2(attr).unwrap();
        let output = event_processor_impl(args, input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }
    #[test]
    fn test_attr_parsing() {
        let tokens = quote! {};
        let ha = syn::parse2::<HandlerArgs>(tokens);
        assert!(ha.is_ok());
    }
    #[test]
    fn test_attr_parsing_empty() -> anyhow::Result<()> {
        let tokens = quote! {};
        let ha = syn::parse2::<HandlerArgs>(tokens)?;
        assert!(ha.event_ident.is_none());
        assert!(ha.inherit_expr.is_none());
        Ok(())
    }
    #[test]
    fn test_attr_parsing_event_name() -> anyhow::Result<()> {
        let tokens = quote! {SomeEvent};
        let ha = syn::parse2::<HandlerArgs>(tokens)?;
        assert!(matches!(ha.event_ident, Some(_)));
        assert!(ha.inherit_expr.is_none());
        Ok(())
    }
    #[test]
    fn test_attr_parsing_inherit_ident() -> anyhow::Result<()> {
        let tokens = quote! { inherit=Handler };
        let ha = syn::parse2::<HandlerArgs>(tokens)?;
        assert!(matches!(ha.inherit_expr, Some(_)));
        let e: syn::Expr = ha.inherit_expr.unwrap();

        assert!(e.to_token_stream().to_string() == quote!(Handler).to_string());
        Ok(())
    }
    #[test]
    fn test_attr_parsing_list() -> anyhow::Result<()> {
        let tokens = quote! { SomeEvent,inherit=self.inherit };
        let ha = syn::parse2::<HandlerArgs>(tokens)?;
        assert!(matches!(ha.inherit_expr, Some(_)));
        assert!(matches!(ha.event_ident, Some(_)));
        let e: syn::Expr = ha.inherit_expr.unwrap();
        let ident = ha.event_ident.unwrap();
        let comp = syn::parse2::<syn::Expr>(quote! {self.inherit})?;
        assert!(comp.to_token_stream().to_string() == e.to_token_stream().to_string());
        assert!(ident.to_string() == String::from("SomeEvent"));
        Ok(())
    }
    #[test]
    fn test_attr_with_envelope() -> anyhow::Result<()> {
        let tokens = quote! { SomeEvent,inherit=self.inherit,envelope=MyCustomEnvelope };
        let ha = syn::parse2::<HandlerArgs>(tokens)?;
        assert!(matches!(ha.event_envelope, Some(_)));
        let ident: syn::Ident = ha.event_envelope.unwrap();
        assert!(ident.to_string() == String::from("MyCustomEnvelope"));
        Ok(())
    }
}
