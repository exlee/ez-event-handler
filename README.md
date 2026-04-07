# ez-event-handler

Makes event and handler definitions easy and without boiler plate.

- Defines Event structs directly from handler definitions
- Auto-implements routing function
- Auto-implements event constructors
- Provides run() function given tokio receiver
- Tracing through Envelope (containing span and id)
- Supports mocking through inheritance of handlers
- Functions all well formed so static analysis works as it should

## Example

```rust
struct EventHandler {}

#[event_handler]
impl EventHandler {
    #[handler(UserAuthentication)]
    fn authenticate_user(&self, username: String, password: String) {
        println!("Authenticating user: {} with provided credentials", username);
    }
    #[handler(DataValidation)]
    fn validate_data(&self, data: Vec<String>, is_valid: bool) {
        println!("Validating data with {} items and validity status: {}", data.len(), is_valid);
    }
    #[handler(ResourceCleanup)]
    fn cleanup_resources(&self, resources: Vec<&str>, force: bool) {
        println!("Cleaning up {} resources with force flag: {}", resources.len(), force);
    }
    #[handler(NotificationSending)]
    fn send_notification(&self, recipient: &str, message: String) {
        println!("Sending notification to {} with message: {}", recipient, message);
    }
}
```

Into

```rust
pub struct EventEnvelope {
    pub event: Event,
    pub span: ::tracing::Span,
    pub id: ::uuid::Uuid,
}
impl EventEnvelope {
    pub fn with_span(self, span: &::tracing::Span) -> Self {
        Self { span: span.clone(), ..self }
    }
}
#[derive(::serde::Serialize, ::serde::Deserialize, Clone)]
pub enum Event {
    UserAuthentication { username: String, password: String },
    DataValidation { data: Vec<String>, is_valid: bool },
    ResourceCleanup { resources: Vec<&str>, force: bool },
    NotificationSending { recipient: &str, message: String },
}
impl Event {
    pub fn new_user_authentication(username: String, password: String) -> EventEnvelope {
        return EventEnvelope {
            span: ::tracing::Span::current(),
            id: ::uuid::Uuid::new_v4(),
            event: Event::UserAuthentication {
                username,
                password,
            },
        };
    }
    pub fn new_data_validation(data: Vec<String>, is_valid: bool) -> EventEnvelope {
        return EventEnvelope {
            span: ::tracing::Span::current(),
            id: ::uuid::Uuid::new_v4(),
            event: Event::DataValidation {
                data,
                is_valid,
            },
        };
    }
    pub fn new_resource_cleanup(resources: Vec<&str>, force: bool) -> EventEnvelope {
        return EventEnvelope {
            span: ::tracing::Span::current(),
            id: ::uuid::Uuid::new_v4(),
            event: Event::ResourceCleanup {
                resources,
                force,
            },
        };
    }
    pub fn new_notification_sending(recipient: &str, message: String) -> EventEnvelope {
        return EventEnvelope {
            span: ::tracing::Span::current(),
            id: ::uuid::Uuid::new_v4(),
            event: Event::NotificationSending {
                recipient,
                message,
            },
        };
    }
}
impl From<Event> for EventEnvelope {
    fn from(input: Event) -> EventEnvelope {
        EventEnvelope {
            span: ::tracing::Span::current(),
            id: ::uuid::Uuid::new_v4(),
            event: input,
        }
    }
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::UserAuthentication { .. } => write!(f, stringify!(UserAuthentication)),
            Event::DataValidation { .. } => write!(f, stringify!(DataValidation)),
            Event::ResourceCleanup { .. } => write!(f, stringify!(ResourceCleanup)),
            Event::NotificationSending { .. } => {
                write!(f, stringify!(NotificationSending))
            }
            _ => write!(f, "Unknown Event"),
        }
    }
}
impl EventHandler {
    async fn authenticate_user(
        &self,
        env: EventEnvelope,
        #[allow(unused)]
        queue: &mut ::std::collections::VecDeque<EventEnvelope>,
    ) {
        let Event::UserAuthentication { username, password } = env.event else {
            unreachable!();
        };
        {
            println!("Authenticating user: {} with provided credentials", username);
        }
    }
    async fn validate_data(
        &self,
        env: EventEnvelope,
        #[allow(unused)]
        queue: &mut ::std::collections::VecDeque<EventEnvelope>,
    ) {
        let Event::DataValidation { data, is_valid } = env.event else {
            unreachable!();
        };
        {
            println!(
                "Validating data with {} items and validity status: {}", data.len(),
                is_valid
            );
        }
    }
    async fn cleanup_resources(
        &self,
        env: EventEnvelope,
        #[allow(unused)]
        queue: &mut ::std::collections::VecDeque<EventEnvelope>,
    ) {
        let Event::ResourceCleanup { resources, force } = env.event else {
            unreachable!();
        };
        {
            println!(
                "Cleaning up {} resources with force flag: {}", resources.len(), force
            );
        }
    }
    async fn send_notification(
        &self,
        env: EventEnvelope,
        #[allow(unused)]
        queue: &mut ::std::collections::VecDeque<EventEnvelope>,
    ) {
        let Event::NotificationSending { recipient, message } = env.event else {
            unreachable!();
        };
        {
            println!("Sending notification to {} with message: {}", recipient, message);
        }
    }
    fn __event_preprocess(&self, _event: &mut Event) {}
    #[tracing::instrument(skip_all, parent = &env.span)]
    pub async fn handle(
        &self,
        mut env: EventEnvelope,
        queue: &mut ::std::collections::VecDeque<EventEnvelope>,
    ) {
        use Event::*;
        tracing::debug!("handle: {:?}", & env.event);
        self.__event_preprocess(&mut env.event);
        match env.event {
            Event::UserAuthentication { .. } => self.authenticate_user(env, queue).await,
            Event::DataValidation { .. } => self.validate_data(env, queue).await,
            Event::ResourceCleanup { .. } => self.cleanup_resources(env, queue).await,
            Event::NotificationSending { .. } => self.send_notification(env, queue).await,
        };
    }
    pub async fn run(&self, mut rx: ::tokio::sync::mpsc::Receiver<EventEnvelope>) {
        let mut queue: ::std::collections::VecDeque<EventEnvelope> = ::std::collections::VecDeque::new();
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
}

```
