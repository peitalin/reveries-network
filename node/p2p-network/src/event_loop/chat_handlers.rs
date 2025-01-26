use crate::behaviour::ChatMessage;
use runtime::tee_attestation;
use super::EventLoop;

impl EventLoop {

    pub(super) async fn broadcast_chat_message(&mut self, message: ChatMessage) {

        self.log(format!("Broadcasting message: {:?}", message));

        /////////// form heartbeat payload here. Move it later
        self.swarm.behaviour_mut().heartbeat.increment_block_height();

        let (
            _tee_quote ,
            tee_quote_bytes
        ) = tee_attestation::generate_tee_attestation()
            .expect("tee attestation generation err");

        // self.swarm.behaviour_mut().heartbeat.set_tee_attestation(
        //     format!("0x{}", hex::encode(tee_attestation_quote.signature.quote_signature))
        // );
        self.swarm.behaviour_mut().heartbeat.set_tee_attestation(tee_quote_bytes);
        //////////

        match self.topics.get(&message.topic.to_string()) {
            Some(topic) => {

                // self.swarm.behaviour_mut().heartbeat.

                match self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), message.message) {
                        Ok(_) => {}
                        Err(e) => println!("Publish err: {:?}", e),
                    }
            }
            None => {
                self.log(format!("Err: topic does not exist {:?}", message.topic));
                self.log(format!("Topics subscribed:"));
                let _ = self.swarm.behaviour_mut().gossipsub.topics()
                    .into_iter()
                    .map(|t| println!("{:?}", t)).collect::<()>();
            }
        }
    }
}