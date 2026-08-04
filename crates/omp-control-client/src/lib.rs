#![forbid(unsafe_code)]
#![doc = "Cross-platform reconnecting client for the OMP control protocol."]

mod adapter;
#[cfg(target_arch = "wasm32")]
mod browser;
mod client;
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod reducer;
mod storage;

pub use adapter::*;
#[cfg(target_arch = "wasm32")]
pub use browser::*;
pub use client::*;
pub use config::*;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
pub use reducer::*;
pub use storage::*;

#[cfg(not(target_arch = "wasm32"))]
impl<S> ClientRunner<S>
where
    S: CredentialStore + Send + 'static,
{
    pub fn spawn_native(self) -> tokio::task::JoinHandle<Result<(), ClientRunError>> {
        tokio::spawn(self.run(NativeWebSocketAdapter))
    }
}

#[cfg(target_arch = "wasm32")]
impl<S> ClientRunner<S>
where
    S: CredentialStore + 'static,
{
    pub fn spawn_browser(self) -> futures_channel::oneshot::Receiver<Result<(), ClientRunError>> {
        let (sender, receiver) = futures_channel::oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = sender.send(self.run(BrowserWebSocketAdapter).await);
        });
        receiver
    }
}
