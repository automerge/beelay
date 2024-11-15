//! This module implements a simple handshake protocol for point-to-point connections
//!
//! The underlying messages of the beelay protocol which are passed to
//! [`crate::Beelay::handle_event`] require that you already know the peer ID of the sender of the
//! message. In many cases (e.g. a new TCP connection) you don't know anything about the other peer
//! except that they have connected and so a simple handshake protocol is needed in which the two
//! peers exchange their peer IDs before they can start sending messages. This module implements
//! such a protocol.
//!
//! To use this module you first create a [`Connecting`] object using either [`Connecting::accept`]
//! if you are the party being connected to or [`Connecting::connect`] if you are the party
//! initiating the connection. Then you loop, calling [`Connecting::receive`] with any message the
//! other end has sent. Each call to [`Connecting::receive`] will return a [`Step`] which tells you
//! whether the handshake is complete and if so, what the peer IDs of the two parties are.
//!
//! Once the handshake is complete you will have a [`Connected`] object, which you can use to
//! transform incoming [`Message`]s into [`crate::Envelope`]s which can be passed to
//! [`crate::Beelay::handle_event`] and to transform outgoing [`crate::Envelope`]s into
//! [`Message`]s which can be sent to the other party.
//!
//! # Example
//!
//! In the following example we make use of a pretend network which we model like this:
//!
//! ```rust
//! fn receive_message() -> Vec<u8> {
//!    vec![]
//! }
//!
//! fn send_message(msg: Vec<u8>) {
//! }
//! ```
//!
//! ```rust,no_run
//! use beelay_core::messages::stream::{Connecting, Connected, Step, Message};
//! use beelay_core::{Beelay, Envelope, Event, PeerId};
//! # fn receive_message() -> Vec<u8> {
//! #    vec![]
//! # }
//! # fn send_message(msg: Vec<u8>) {
//! # }
//!
//! fn accept_connection(our_peer_id: PeerId) {
//!     let step = Connecting::accept(our_peer_id);
//!     let connected = handshake(step);
//!     run(connected);
//! }
//!
//! fn connect_to_peer(our_peer_id: PeerId) {
//!     let step = Connecting::connect(our_peer_id);
//!     let connected = handshake(step);
//!     run(connected);
//! }
//!
//! fn handshake(mut step: Step) -> Connected {
//!     loop {
//!         match step {
//!             Step::Continue(state, msg) => {
//!                 if let Some(msg) = msg {
//!                     send_message(msg.encode());
//!                 }
//!                 let next_msg = receive_message();
//!                 step = state.receive(Message::decode(&next_msg).unwrap()).unwrap();
//!             },
//!             Step::Done(connected, msg) => {
//!                 if let Some(msg) = msg {
//!                     send_message(msg.encode());
//!                 }
//!                 break connected;
//!             }
//!         }
//!     }
//! }
//!
//! fn run(connected: Connected) {
//!     // Now we can start sending and receiving messages
//!
//!     // We can translate incoming messages into an envelope to give to Beelay
//!     let incoming = receive_message();
//!     let msg = Message::decode(&incoming).unwrap();
//!     let envelope = connected.receive(msg).unwrap();
//!     let beelay: Beelay::<rand::rngs::OsRng> = todo!();
//!     beelay.handle_event(Event::receive(envelope));
//!     println!("Received message from {}: {:?}", envelope.sender(), envelope.payload());
//!
//!     // A message somehow generated by an instance of Beelay in our application
//!     let envelope: Envelope = todo!();
//!     let msg = connected.send(envelope);
//!     send_message(msg.encode());
//! }
//! ```
use crate::{leb128::encode_uleb128, parse, Envelope, Payload, PeerId};
pub use error::{DecodeError, Error};

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
pub struct Message(MessageInner);

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let msg_type = match &self.0 {
            MessageInner::HelloDearServer(_) => 0,
            MessageInner::WhyHelloDearClient(_) => 1,
            MessageInner::Data(_) => 2,
        };
        let mut bytes = vec![msg_type];
        match &self.0 {
            MessageInner::HelloDearServer(peer_id) => {
                encode_uleb128(&mut bytes, peer_id.as_bytes().len() as u64);
                bytes.extend_from_slice(peer_id.as_bytes());
            }
            MessageInner::WhyHelloDearClient(peer_id) => {
                encode_uleb128(&mut bytes, peer_id.as_bytes().len() as u64);
                bytes.extend_from_slice(peer_id.as_bytes());
            }
            MessageInner::Data(payload) => bytes.extend_from_slice(&payload.encode()),
        }
        bytes
    }

    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        let input = parse::Input::new(data);
        let (input, msg_type) = parse::u8(input)?;
        match msg_type {
            0 => {
                let (_input, peer_id_str) = parse::str(input)?;
                let peer_id = PeerId::from(peer_id_str.to_string());
                Ok(Message(MessageInner::HelloDearServer(peer_id)))
            }
            1 => {
                let (_input, peer_id_str) = parse::str(input)?;
                let peer_id = PeerId::from(peer_id_str.to_string());
                Ok(Message(MessageInner::WhyHelloDearClient(peer_id)))
            }
            2 => {
                let (_input, payload) = crate::messages::decode::parse_payload(input)?;
                Ok(Message(MessageInner::Data(payload)))
            }
            _ => Err(DecodeError::Invalid("invalid message type".to_string())),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
enum MessageInner {
    HelloDearServer(PeerId),
    WhyHelloDearClient(PeerId),
    Data(Payload),
}

/// The initial state of the handshake protocol.
pub struct Connecting(PeerId);

/// A step in the handshakeprotocol
pub enum Step {
    /// Continue with the handshake. If the optional message is `Some` then it should be sent to
    /// the other end before waiting to receive another message.
    Continue(Connecting, Option<Message>),
    /// The handshake is complete. The `Connected` object contains the peer IDs of the two parties
    /// and if the optional message is `Some` then it should be sent to the other end.
    Done(Connected, Option<Message>),
}

impl Connecting {
    /// A handshake for accepting a connection. This will wait for the other end to send the first
    /// message
    ///
    /// # Arguments
    /// * `us` - The peer ID of the party accepting the connection
    pub fn accept(us: PeerId) -> Step {
        Step::Continue(Connecting(us), None)
    }

    /// A handshake for initiating a connection, this will send the first message.
    ///
    /// # Arguments
    /// * `us` - The peer ID of the party initiating the connection
    pub fn connect(us: PeerId) -> Step {
        Step::Continue(
            Connecting(us.clone()),
            Some(Message(MessageInner::HelloDearServer(us))),
        )
    }

    /// Receive a message from the other end.
    pub fn receive(self, msg: Message) -> Result<Step, Error> {
        match msg.0 {
            MessageInner::HelloDearServer(their_peer_id) => Ok(Step::Done(
                Connected {
                    our_peer_id: self.0.clone(),
                    their_peer_id,
                },
                Some(Message(MessageInner::WhyHelloDearClient(self.0))),
            )),
            MessageInner::WhyHelloDearClient(their_peer_id) => Ok(Step::Done(
                Connected {
                    our_peer_id: self.0,
                    their_peer_id,
                },
                None,
            )),
            _ => Err(Error::UnexpectedMessage),
        }
    }
}

/// The connected state of the handshake protocol
#[derive(Clone)]
pub struct Connected {
    our_peer_id: PeerId,
    their_peer_id: PeerId,
}

impl Connected {
    pub fn their_peer_id(&self) -> &PeerId {
        &self.their_peer_id
    }

    /// Receive a message from the other end and transform it into an envelope
    pub fn receive(&self, msg: Message) -> Result<Envelope, Error> {
        match msg.0 {
            MessageInner::Data(payload) => Ok(Envelope {
                sender: self.their_peer_id.clone(),
                recipient: self.our_peer_id.clone(),
                payload,
            }),
            _ => Err(Error::UnexpectedMessage),
        }
    }

    /// Transform an envelope into a message which can be sent to the other end
    pub fn send(&self, env: Envelope) -> Message {
        Message(MessageInner::Data(env.take_payload()))
    }
}

mod error {
    use crate::parse;

    pub enum Error {
        UnexpectedMessage,
    }

    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            match self {
                Error::UnexpectedMessage => write!(f, "unexpected message"),
            }
        }
    }

    impl std::fmt::Debug for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(self, f)
        }
    }

    impl std::error::Error for Error {}

    pub enum DecodeError {
        NotEnoughInput,
        Invalid(String),
    }

    impl From<parse::ParseError> for DecodeError {
        fn from(err: parse::ParseError) -> Self {
            match err {
                parse::ParseError::NotEnoughInput => DecodeError::NotEnoughInput,
                parse::ParseError::Other { context, error } => {
                    DecodeError::Invalid(format!("{:?}: {}", context, error))
                }
            }
        }
    }

    impl std::fmt::Display for DecodeError {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            match self {
                DecodeError::NotEnoughInput => write!(f, "not enough input"),
                DecodeError::Invalid(msg) => write!(f, "invalid input: {}", msg),
            }
        }
    }

    impl std::fmt::Debug for DecodeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(self, f)
        }
    }

    impl std::error::Error for DecodeError {}
}

#[cfg(test)]
mod tests {

    #[test]
    fn handshake_message_encoding_roundtrip() {
        bolero::check!()
            .with_arbitrary::<super::Message>()
            .for_each(|msg| {
                let encoded = msg.encode();
                let decoded = super::Message::decode(&encoded).unwrap();
                assert_eq!(msg, &decoded);
            });
    }
}