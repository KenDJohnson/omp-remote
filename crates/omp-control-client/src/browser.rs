use std::{
    cell::RefCell,
    future::{Future, Ready, ready},
    net::IpAddr,
    pin::Pin,
    rc::Rc,
    time::Duration,
};

use futures_channel::{mpsc, oneshot};
use futures_util::StreamExt;
use js_sys::{ArrayBuffer, Uint8Array};
use omp_control_protocol::TlsIdentityHint;
use url::Url;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{BinaryType, CloseEvent, ErrorEvent, Event, MessageEvent, WebSocket};

use crate::{BinaryWebSocket, SocketEvent, SocketTarget, TransportError, WebSocketAdapter};

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Clone, Copy, Debug, Default)]
pub struct BrowserWebSocketAdapter;

pub struct BrowserWebSocket {
    socket: WebSocket,
    incoming: mpsc::UnboundedReceiver<Result<SocketEvent, TransportError>>,
    _on_open: Closure<dyn FnMut(Event)>,
    _on_error: Closure<dyn FnMut(ErrorEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

impl WebSocketAdapter for BrowserWebSocketAdapter {
    type Socket = BrowserWebSocket;
    type ConnectFuture<'a> = LocalBoxFuture<'a, Result<Self::Socket, TransportError>>;
    type SleepFuture<'a> = gloo_timers::future::TimeoutFuture;

    fn connect<'a>(&'a self, target: &'a SocketTarget) -> Self::ConnectFuture<'a> {
        Box::pin(async move {
            validate_target(target)?;
            let socket = WebSocket::new(&target.endpoint)
                .map_err(|_| TransportError::new("browser rejected the WebSocket endpoint"))?;
            socket.set_binary_type(BinaryType::Arraybuffer);

            let (incoming_tx, incoming) = mpsc::unbounded();
            let (open_tx, open_rx) = oneshot::channel();
            let open_tx = Rc::new(RefCell::new(Some(open_tx)));

            let open_result = Rc::clone(&open_tx);
            let on_open = Closure::new(move |_event: Event| {
                if let Some(sender) = open_result.borrow_mut().take() {
                    let _ = sender.send(Ok(()));
                }
            });
            socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

            let error_result = Rc::clone(&open_tx);
            let error_incoming = incoming_tx.clone();
            let on_error = Closure::new(move |_event: ErrorEvent| {
                let error = TransportError::new("browser WebSocket reported a network error");
                if let Some(sender) = error_result.borrow_mut().take() {
                    let _ = sender.send(Err(error.clone()));
                }
                let _ = error_incoming.unbounded_send(Err(error));
            });
            socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

            let close_result = Rc::clone(&open_tx);
            let close_incoming = incoming_tx.clone();
            let on_close = Closure::new(move |event: CloseEvent| {
                if let Some(sender) = close_result.borrow_mut().take() {
                    let _ = sender.send(Err(TransportError::new(
                        "browser WebSocket closed before opening",
                    )));
                }
                let reason = (!event.reason().is_empty()).then(|| event.reason());
                let _ = close_incoming.unbounded_send(Ok(SocketEvent::Closed { reason }));
            });
            socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

            let message_incoming = incoming_tx;
            let on_message = Closure::new(move |event: MessageEvent| {
                let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() else {
                    let _ = message_incoming.unbounded_send(Err(TransportError::new(
                        "browser WebSocket received a non-binary message",
                    )));
                    return;
                };
                let bytes = Uint8Array::new(&buffer).to_vec();
                let _ = message_incoming.unbounded_send(Ok(SocketEvent::Binary(bytes)));
            });
            socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

            let connection = BrowserWebSocket {
                socket,
                incoming,
                _on_open: on_open,
                _on_error: on_error,
                _on_close: on_close,
                _on_message: on_message,
            };
            open_rx
                .await
                .map_err(|_| TransportError::new("browser WebSocket open signal was dropped"))??;
            Ok(connection)
        })
    }

    fn sleep(&self, duration: Duration) -> Self::SleepFuture<'_> {
        let milliseconds = duration.as_millis().min(u128::from(u32::MAX)) as u32;
        gloo_timers::future::TimeoutFuture::new(milliseconds)
    }
}

impl BinaryWebSocket for BrowserWebSocket {
    type SendFuture<'a> = Ready<Result<(), TransportError>>;
    type ReceiveFuture<'a> = LocalBoxFuture<'a, Result<SocketEvent, TransportError>>;
    type CloseFuture<'a> = Ready<Result<(), TransportError>>;

    fn send_binary(&mut self, bytes: Vec<u8>) -> Self::SendFuture<'_> {
        ready(
            self.socket
                .send_with_u8_array(&bytes)
                .map_err(|_| TransportError::new("browser failed to send WebSocket frame")),
        )
    }

    fn receive(&mut self) -> Self::ReceiveFuture<'_> {
        Box::pin(async move {
            self.incoming
                .next()
                .await
                .unwrap_or(Ok(SocketEvent::Closed { reason: None }))
        })
    }

    fn close(&mut self) -> Self::CloseFuture<'_> {
        ready(
            self.socket
                .close()
                .map_err(|_| TransportError::new("browser failed to close WebSocket")),
        )
    }
}

impl Drop for BrowserWebSocket {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        self.socket.set_onmessage(None);
    }
}

fn validate_target(target: &SocketTarget) -> Result<(), TransportError> {
    let url = Url::parse(&target.endpoint)
        .map_err(|error| TransportError::new(format!("invalid WebSocket endpoint: {error}")))?;
    match target.tls_identity {
        TlsIdentityHint::PubliclyTrusted => {
            if url.scheme() != "wss" {
                return Err(TransportError::new(
                    "browser trusted connections require a wss:// endpoint",
                ));
            }
        }
        TlsIdentityHint::InsecureDevelopment => {
            if url.scheme() != "ws" || !is_loopback(&url) {
                return Err(TransportError::new(
                    "browser development connections require a loopback ws:// endpoint",
                ));
            }
        }
        TlsIdentityHint::Sha256Fingerprint(_) => {
            return Err(TransportError::new(
                "browsers do not expose certificate pinning; use a publicly trusted certificate",
            ));
        }
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}
