use proc_macro::TokenStream;
mod event_handler;
#[proc_macro_attribute]
pub fn event_processor(args: TokenStream, input: TokenStream) -> TokenStream {
    let handler_args = syn::parse_macro_input!(args as event_handler::HandlerArgs);
    event_handler::event_processor_impl(handler_args, input.into()).into()
}
