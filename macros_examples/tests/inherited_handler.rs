use std::collections::VecDeque;

use event_handler_macro::event_processor;

struct DefaultHandler {}

#[event_processor(Event)]
impl DefaultHandler {
    #[handler(EventA)]
    fn process_a(&self, number: u32) {
        println!("A{}", number);
    }
    #[handler(EventB)]
    #[impure]
    fn process_b(&self, number: u32) {
        println!("B{}", number);
    }
}

struct TestHandler {
    pub inner: DefaultHandler,
}
#[event_processor(Event,inherit=self.inner)]
impl TestHandler {
    #[handler(EventA)]
    fn process_a(&self, number: u32) {
        println!("TEST: A{}", number);
    }
    #[handler(EventB)]
    fn process_b(&self, number: u32) {
        println!("TEST: B{}", number);
    }
}

#[tokio::test]
async fn example() {
    let h1 = DefaultHandler {};
    let h2 = DefaultHandler {};
    let t = TestHandler { inner: h2 };
    // let e_a: EventEnvelope = Event::EventA {
    //     number: 10,
    //     id: uuid::Uuid::new_v4(),
    // }
    // .into();
    let e_a = Event::new_event_a(10);
    let e_b = Event::new_event_b(20);
    let mut vd = VecDeque::<EventEnvelope>::new();

    h1.handle(e_a, &mut vd).await;
    h1.handle(e_b, &mut vd).await; // fails at runtime

    let e_a: EventEnvelope = Event::EventA {
        number: 10,
        id: uuid::Uuid::new_v4(),
    }
    .into();
    let e_b: EventEnvelope = Event::EventB {
        number: 20,
        id: uuid::Uuid::new_v4(),
    }
    .into();
    t.handle(e_a, &mut vd).await;
    t.handle(e_b, &mut vd).await; // mocked - runs fine
}
