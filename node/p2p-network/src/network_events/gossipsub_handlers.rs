use color_eyre::Result;
use libp2p::gossipsub;
use super::NetworkEvents;

impl<'a> NetworkEvents<'a> {
    pub async fn handle_gossipsub_event(&mut self, gevent: gossipsub::Event) -> Result<()> {
        match gevent {
            gossipsub::Event::Message { ..  } => {}
            gossipsub::Event::Subscribed { peer_id, topic } => {}
            gossipsub::Event::Unsubscribed { peer_id, topic } => {}
            gossipsub::Event::GossipsubNotSupported {..} => {}
            gossipsub::Event::SlowPeer {..} => {}
        }
        Ok(())
    }
}

