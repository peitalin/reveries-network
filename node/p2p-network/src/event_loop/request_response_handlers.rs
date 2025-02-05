use libp2p::request_response;
use crate::SendError;
use crate::types::{
    NetworkLoopEvent,
    FragmentRequest,
    FragmentResponse,
};
use super::EventLoop;


impl EventLoop {
    pub(super) async fn handle_request_response(
        &mut self,
        event: request_response::Event<FragmentRequest, FragmentResponse>,
    ) {
        match event {
            //// Request Response Protocol
            request_response::Event::Message { message, .. } => {
                match message {
                    request_response::Message::Request {
                        request: fragment_request,
                        channel,
                        ..
                    } => {
                        self.network_event_sender
                            .send(NetworkLoopEvent::InboundCfragRequest {
                                agent_name: fragment_request.0,
                                agent_nonce: fragment_request.1,
                                frag_num: fragment_request.2,
                                sender_peer: fragment_request.3,
                                channel,
                            })
                            .await
                            .expect("Event receiver not to be dropped.");
                    }
                    request_response::Message::Response {
                        request_id,
                        response,
                    } => {

                        let sender = self.pending.request_fragments
                            .remove(&request_id)
                            .expect("request_response: Request pending.");

                        let _ = sender.send(response.0);
                    }
                }
            },
            request_response::Event::InboundFailure { .. } => {
                // self.log(format!("InboundFailure: {:?} {:?} {:?}", peer, request_id, error));
            }
            request_response::Event::OutboundFailure {
                request_id,
                error,
                peer
            } => {
                // self.log(format!("OutboundFailure: {:?} {:?} {:?}", peer, request_id, error));
                let _ = self.pending.request_fragments
                    .remove(&request_id)
                    .expect("Request pending")
                    .send(Err(SendError(error.to_string())));

            }
            request_response::Event::ResponseSent { peer, .. } => {
                // self.log(format!("ResponseSent to {:?}", peer));
            }
        }
    }
}

