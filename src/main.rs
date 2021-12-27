use core::fmt;
use std::{error::Error, io::Read};

use matrix_sdk::{
    self,
    room::Room,
    ruma::{
        self,
        events::{
            reaction::ReactionEventContent,
            room::message::{MessageEventContent, MessageType, TextMessageEventContent},
            AnyMessageEvent, AnyRoomEvent, SyncMessageEvent,
        },
        RoomId,
    },
    Client, SyncSettings,
};

use url::Url;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct RawConfig {
    homeserver: String,
    username: String,
    password: String,
    input_room_id: String,
    mod_room_id: String,
    output_room_id: String,
}

struct Config {
    homeserver: String,
    username: String,
    password: String,
    input_room_id: RoomId,
    mod_room_id: RoomId,
    output_room_id: RoomId,
}

impl TryFrom<RawConfig> for Config {
    type Error = FourwarderError;
    fn try_from(config: RawConfig) -> Result<Self, Self::Error> {
        Ok(Config {
            homeserver: config.homeserver,
            username: config.username,
            password: config.password,
            input_room_id: RoomId::try_from(config.input_room_id.as_str())
                .map_err(|_| FourwarderError::Config("`input_room_id` is not a valid `RoomId`"))?,
            mod_room_id: RoomId::try_from(config.mod_room_id.as_str())
                .map_err(|_| FourwarderError::Config("`mod_room_id` is not a valid `RoomId`"))?,
            output_room_id: RoomId::try_from(config.output_room_id.as_str())
                .map_err(|_| FourwarderError::Config("`output_room_id` is not a valid `RoomId`"))?,
        })
    }
}

use lazy_static::lazy_static;
lazy_static! {
    static ref CONFIG: Config = {
        const CONFIG_LOCATION: &str = "4warder.toml";

        let mut config_file = std::fs::File::open(CONFIG_LOCATION)
            .unwrap_or_else(|_| panic!("Config not found, looking for {}", CONFIG_LOCATION));

        let mut config_raw = String::new();

        config_file
            .read_to_string(&mut config_raw)
            .expect("Could not read config file");

        let config: RawConfig = toml::from_str(&config_raw)
            .unwrap_or_else(|_| panic!("Could not parse config file as valid TOML"));

        config.try_into().unwrap()
    };
}

#[derive(Debug)]
enum FourwarderError {
    Config(&'static str),
    Matrix(matrix_sdk::Error),
    /// A false assumption has been made in the code, but is recoverable
    Logic(&'static str),
}

impl std::fmt::Display for FourwarderError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self {
            Self::Config(err) => write!(f, "{}", err),
            Self::Matrix(err) => write!(f, "{}", err),
            Self::Logic(err) => write!(f, "{}", err),
        }
    }
}

impl Error for FourwarderError {}

impl From<matrix_sdk::Error> for FourwarderError {
    fn from(err: matrix_sdk::Error) -> Self {
        FourwarderError::Matrix(err)
    }
}

async fn on_room_message(
    event: SyncMessageEvent<MessageEventContent>,
    room: Room,
    client: Client,
) -> Result<(), matrix_sdk::Error> {
    if let Room::Joined(room) = room {
        if let SyncMessageEvent {
            content:
                MessageEventContent {
                    msgtype: MessageType::Text(TextMessageEventContent { body: msg_body, .. }),
                    ..
                },
            ..
        } = event
        {
            if room.room_id() == &CONFIG.input_room_id {
                println!("Recieved message in input room, {:?}", msg_body);
                client
                    .room_send(
                        &CONFIG.mod_room_id,
                        MessageEventContent::text_plain(msg_body),
                        None,
                    )
                    .await?;
            }
        }
    }

    Ok(())
}

async fn on_room_react(
    event: SyncMessageEvent<ReactionEventContent>,
    room: Room,
    client: Client,
) -> Result<(), FourwarderError> {
    let reacted_to = event.content.relates_to.event_id;
    let emoji = event.content.relates_to.emoji;

    if emoji == "✅" && room.room_id() == &CONFIG.mod_room_id {
        let mod_room = match client.get_joined_room(&CONFIG.mod_room_id) {
            Some(joined) => joined,
            None => {
                return Err(FourwarderError::Logic(
                    "We could not get a joined room that we definetly have joined before",
                ))
            }
        };

        let orig_event = mod_room
            .event(ruma::api::client::r0::room::get_room_event::Request::new(
                &CONFIG.mod_room_id,
                &reacted_to,
            ))
            .await
            .map_err(FourwarderError::Matrix)?
            .event
            .deserialize();

        let orig_event =
            orig_event.map_err(|e| FourwarderError::Matrix(matrix_sdk::Error::SerdeJson(e)))?;

        // This mess of destructuring assignment gets us to the body of the message the reaction is for
        if let AnyRoomEvent::Message(AnyMessageEvent::RoomMessage(msg)) = orig_event {
            match msg.content.msgtype {
                MessageType::Text(TextMessageEventContent { ref body, .. }) => {
                    client
                        .room_send(
                            &CONFIG.output_room_id,
                            MessageEventContent::text_plain(body), // send the text unaltered
                            None,
                        )
                        .await
                        .map_err(FourwarderError::Matrix)?;
                }
                _ => {
                    return Err(FourwarderError::Logic(
                        "We assumed that the message being reacted to was a text message",
                    ));
                }
            }
        };
    }
    Ok(())
}

/// Log into the homesever, sync the client and register event handlers
///
/// This function will never return, as [`matrix_sdk::Client::sync`] never returns.
async fn login_and_sync(
    homeserver_url: &str,
    username: &str,
    password: &str,
) -> Result<(), FourwarderError> {
    let homeserver_url = Url::parse(homeserver_url)
        .map_err(|e| FourwarderError::Matrix(matrix_sdk::Error::Url(e)))?;
    let client = Client::new(homeserver_url)?;

    client
        .login(username, password, None, Some("4warder_bot"))
        .await?;

    client.sync_once(SyncSettings::default()).await?;

    // Create a list of rooms we have been invited to that we are going to use
    let rooms_to_join = client.invited_rooms().into_iter().filter(|el| {
        el.room_id() == &CONFIG.input_room_id
            || el.room_id() == &CONFIG.mod_room_id
            || el.room_id() == &CONFIG.output_room_id
    });
    // Join the rooms we just picked
    for room in rooms_to_join {
        room.accept_invitation().await?;
    }

    client.register_event_handler(on_room_message).await;

    client.register_event_handler(on_room_react).await;

    let settings = SyncSettings::default().token(match client.sync_token().await {
        Some(s) => s,
        None => {
            return Err(FourwarderError::Logic(
                "Could not get sync token... if we don't have it now, what's going on?",
            ))
        }
    });

    // This function will never return
    client.sync(settings).await;

    Err(FourwarderError::Logic("`client.sync` returned."))
}

#[tokio::main]
async fn main() -> Result<(), FourwarderError> {
    tracing_subscriber::fmt::init();

    let (homeserver_url, username, password) = (
        CONFIG.homeserver.as_str(),
        CONFIG.username.as_str(),
        CONFIG.password.as_str(),
    );

    println!(
        "Launching 4warder_bot on {} as {}",
        homeserver_url, username
    );

    login_and_sync(homeserver_url, username, password).await?;

    Ok(())
}
