use std::{path::PathBuf, sync::mpsc, thread};
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::{EventLoopBuilderExtWindows, WindowBuilderExtWindows},
    window::WindowBuilder,
};
use tokio::sync::mpsc::UnboundedSender;
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

pub const PAGE: &str = include_str!("webplayer.html");

pub struct WebPlayer {
    proxy: EventLoopProxy<String>,
}

impl WebPlayer {
    pub fn spawn(url: String, data_dir: PathBuf, events: UnboundedSender<String>) -> anyhow::Result<Self> {
        let (ready_tx, ready_rx) = mpsc::channel();

        thread::Builder::new().name("webplayer".into()).spawn(move || {
            let event_loop = EventLoopBuilder::<String>::with_user_event().with_any_thread(true).build();
            let window = match WindowBuilder::new()
                .with_title("Rustify Player")
                .with_visible(false)
                .with_skip_taskbar(true)
                .build(&event_loop)
            {
                Ok(window) => window,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("player window: {e}")));
                    return;
                }
            };

            let origin = url.split("/webplayer").next().unwrap_or_default().to_owned();
            let allowed = format!("{origin}/webplayer");
            let mut context = WebContext::new(Some(data_dir));
            let webview = match WebViewBuilder::new_with_web_context(&mut context)
                .with_url(&url)
                .with_autoplay(true)
                .with_initialization_script(include_str!("audio.js"))
                .with_devtools(false)
                .with_navigation_handler(move |target| target == allowed || target.starts_with(&format!("{allowed}?")))
                .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
                .with_ipc_handler(move |request| {
                    let from = request.uri();
                    let same = format!("{}://{}", from.scheme_str().unwrap_or_default(), from.authority().map(|a| a.as_str()).unwrap_or_default());
                    if same == origin {
                        let _ = events.send(request.into_body());
                    }
                })
                .build(&window)
            {
                Ok(webview) => webview,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("player webview: {e}")));
                    return;
                }
            };

            let _ = ready_tx.send(Ok(event_loop.create_proxy()));
            event_loop.run(move |event, _, flow| {
                *flow = ControlFlow::Wait;
                if let Event::UserEvent(script) = event
                    && let Err(e) = webview.evaluate_script(&script) {
                        tracing::warn!("player script: {e}");
                    }
                let _ = (&window, &context);
            });
        })?;

        let proxy = ready_rx.recv()??;
        Ok(Self { proxy })
    }

    pub fn eval(&self, script: String) {
        if self.proxy.send_event(script).is_err() {
            tracing::warn!("player thread is gone");
        }
    }
}
