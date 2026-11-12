use std::time::Duration;

use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use futures::StreamExt;
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::{Instant, sleep, sleep_until, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info};

use crate::{
    config::SharedCoreConfig,
    events::{Event, TwitchAction},
    handlers::{data::DataBuilder, state::State},
    twitch::{
        context::TwitchWebsocketContext,
        handlers::{
            incoming_message::handle_incoming_message,
            message_commands::handle_command_message,
            send_message::handle_send_message,
            welcome_message::{handle_channel_join, handle_welcome_message},
        },
        models::ReceivedTwitchMessage,
        oauth::TwitchOauth,
    },
};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TwitchWebsocket {}

impl TwitchWebsocket {
    pub fn new(
        config: SharedCoreConfig,
        twitch_oauth: TwitchOauth,
        event_tx: Sender<Event>,
        twitch_rx: Receiver<TwitchAction>,
    ) -> Self {
        let mut context = TwitchWebsocketContext::default();
        context.set_oauth(Some(twitch_oauth));

        let mut actor = TwitchWebsocketThread::new(config, context, event_tx, twitch_rx);
        tokio::task::spawn(async move { actor.run().await });

        Self {}
    }
}

pub type WebsocketStream = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

pub struct TwitchWebsocketThread {
    config: SharedCoreConfig,
    context: TwitchWebsocketContext,
    event_tx: Sender<Event>,
    action_rx: Receiver<TwitchAction>,
}

impl TwitchWebsocketThread {
    const fn new(
        config: SharedCoreConfig,
        context: TwitchWebsocketContext,
        event_tx: Sender<Event>,
        action_rx: Receiver<TwitchAction>,
    ) -> Self {
        Self {
            config,
            context,
            event_tx,
            action_rx,
        }
    }

    async fn connect(&mut self) -> Result<WebsocketStream> {
        let url = self.config.twitch.config_twitch_websocket_url();
        let (ws_stream, _) = match connect_async(url).await {
            Ok(websocket_connection) => websocket_connection,
            Err(err) => {
                bail!(
                    "Failed to connect to websocket server at {}: {}",
                    self.config.twitch.server,
                    err
                )
            }
        };

        info!("Twitch websocket handshake successful");

        let (_, mut stream) = ws_stream.split();

        // The welcome message is the first text frame, a ping may come before it.
        let message = loop {
            match stream.next().await {
                Some(Ok(Message::Text(message))) => break message,
                Some(Ok(_)) => debug!("Websocket server has pinged you to make sure you're here"),
                _ => bail!("Welcome message from websocket server was not found"),
            }
        };

        handle_welcome_message(&mut self.context, &self.event_tx, message)
            .await
            .context("Failed to handle welcome message")?;

        Ok(stream)
    }

    async fn run(&mut self) -> Result<()> {
        self.context
            .set_emotes_state(self.config.frontend.is_emotes_enabled());

        // If the dashboard is the start state, wait until the user has selected a channel
        // before connecting: Twitch closes a session that has no subscriptions after a few seconds.
        if self.config.terminal.first_state == State::Dashboard {
            debug!("Waiting for user to select channel from dashboard screen");

            loop {
                match self.action_rx.recv().await {
                    Some(TwitchAction::JoinChannel(channel_name)) => {
                        self.context.set_channel_name(Some(channel_name));

                        debug!("User has selected channel from start screen");
                        break;
                    }
                    Some(_) => {}
                    None => return Ok(()),
                }
            }
        } else {
            self.context
                .set_channel_name(Some(self.config.twitch.channel.clone()));
        }

        // Tell the user about the first failure only, retries stay in the log.
        let mut notify = true;

        loop {
            let problem = match timeout(CONNECT_TIMEOUT, self.connect()).await {
                Ok(Ok(stream)) => {
                    notify = true;

                    if !self.read_until_closed(stream).await? {
                        return Ok(());
                    }

                    "Connection to Twitch lost".to_string()
                }
                Ok(Err(err)) => format!("Failed to connect to Twitch: {err:#}"),
                Err(_) => "Timed out connecting to Twitch".to_string(),
            };

            error!("{problem}, retrying in {RECONNECT_DELAY:?}");

            if notify {
                self.event_tx
                    .send(DataBuilder::twitch(format!("{problem}, reconnecting...")).into())
                    .await?;
                notify = false;
            }

            // Keep reading actions while waiting, so a new channel pick replaces one that cannot be joined.
            tokio::select! {
                () = sleep(RECONNECT_DELAY) => {}
                action = self.action_rx.recv() => match action {
                    Some(TwitchAction::JoinChannel(channel_name)) => {
                        self.context.set_channel_name(Some(channel_name));
                        notify = true;
                    }
                    Some(action) => self.run_twitch_action(action).await?,
                    None => return Ok(()),
                },
            }
        }
    }

    /// Reads chat until the connection is lost. Returns `false` when the app itself has gone away.
    async fn read_until_closed(&mut self, mut stream: WebsocketStream) -> Result<bool> {
        // Twitch sends at least a keepalive this often, a longer silence means the connection is dead.
        let keepalive = Duration::from_secs(
            u64::try_from(self.config.twitch.keepalive_timeout_seconds).unwrap_or(30) + 10,
        );
        let mut deadline = Instant::now() + keepalive;

        loop {
            tokio::select! {
                biased;

                action = self.action_rx.recv() => {
                    let Some(action) = action else {
                        return Ok(false);
                    };

                    self.run_twitch_action(action).await?;
                }
                message = stream.next() => {
                    let message = match message {
                        Some(Ok(message)) if !message.is_close() => message,
                        other => {
                            error!("Twitch connection closed: {other:?}");
                            return Ok(true);
                        }
                    };

                    deadline = Instant::now() + keepalive;

                    if let Err(err) = self.handle_websocket_stream_message(message).await {
                        error!("Failed to handle websocket message: {err}");
                    }
                }
                () = sleep_until(deadline) => {
                    error!("No keepalive from Twitch for {keepalive:?}");
                    return Ok(true);
                }
            };
        }
    }

    /// Runs a user action. A failure is shown in chat, not only logged.
    async fn run_twitch_action(&mut self, action: TwitchAction) -> Result<()> {
        if let Err(err) = self.handle_twitch_action(action).await {
            error!("Failed to handle twitch action: {err:#}");

            self.event_tx
                .send(DataBuilder::twitch(format!("{err:#}")).into())
                .await?;
        }

        Ok(())
    }

    async fn handle_twitch_action(&mut self, action: TwitchAction) -> Result<()> {
        match action {
            TwitchAction::Message(message) => {
                if let Some(command) = message.strip_prefix('/') {
                    handle_command_message(&self.context, &self.event_tx, command)
                        .await
                        .context("Failed to handle Twitch message command from terminal")?;
                } else {
                    handle_send_message(&self.context, message, None)
                        .await
                        .context("Failed to send message")?;
                }
            }
            TwitchAction::Reply { parent_id, message } => {
                handle_send_message(&self.context, message, Some(parent_id))
                    .await
                    .context("Failed to send reply")?;
            }
            TwitchAction::JoinChannel(channel_name) => {
                handle_channel_join(
                    &mut self.context,
                    &self.event_tx,
                    channel_name.clone(),
                    false,
                )
                .await
                .with_context(|| format!("Failed to join #{channel_name}"))?;
            }
        }

        Ok(())
    }

    async fn handle_websocket_stream_message(&self, message: Message) -> Result<()> {
        let Message::Text(message_text) = message else {
            return Ok(());
        };

        let received_message = serde_json::from_str::<ReceivedTwitchMessage>(&message_text)?;

        handle_incoming_message(
            self.config.clone(),
            &self.context,
            &self.event_tx,
            received_message,
        )
        .await
    }
}
