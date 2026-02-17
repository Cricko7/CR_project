use super::*;

#[derive(Clone)]
pub(super) struct ApiEventHub {
    sender: broadcast::Sender<WsServerEvent>,
}

impl ApiEventHub {
    pub(super) fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
    pub(super) fn publish(&self, event: WsServerEvent) {
        let _ = self.sender.send(event);
    }
    pub(super) fn subscribe(&self) -> broadcast::Receiver<WsServerEvent> {
        self.sender.subscribe()
    }
}
