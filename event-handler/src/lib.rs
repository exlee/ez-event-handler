pub use ez_event_handler_macro::event_processor;

pub async fn spawn<F, ENV>(f: F, size: usize) -> ::tokio::sync::mpsc::Sender<ENV>
where
    ENV: Send + 'static,
    F: FnOnce(
            &mut ::tokio::sync::mpsc::Receiver<ENV>,
        )
            -> ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
{
    let (tx, mut rx) = ::tokio::sync::mpsc::channel::<ENV>(size);
    tokio::spawn(async move {
        f(&mut rx).await;
    });
    tx
}

pub trait EventHandlerInsert {
    type Event;
    fn insert_event(&self, event: Self::Event);
}
