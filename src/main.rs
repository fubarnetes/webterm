extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate tokio_codec;
extern crate tokio_io;
extern crate tokio_pty_process;
#[macro_use]
extern crate log;
extern crate pretty_env_logger;

use actix::*;
use actix_web::{Binary, fs::NamedFile, fs::StaticFiles, server, ws, App, HttpRequest, Result};

use futures::future::Future;

use std::process::Command;
use std::time::{Instant, Duration};
use std::io::Write;

use tokio_codec::{BytesCodec, Decoder, FramedRead};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::io::WriteHalf;
use tokio_pty_process::{AsyncPtyMaster, Child, CommandExt};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

type BytesMut = <BytesCodec as Decoder>::Item;

#[derive(Debug)]
struct IO(BytesMut);

impl Message for IO {
    type Result = ();
}

impl Into<Binary> for IO {
    fn into(self) -> Binary {
        self.0.into()
    }
}

impl AsRef<[u8]> for IO {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<Binary> for IO {
    fn from(b: Binary) -> Self {
        Self(b.as_ref().into())
    }
}

impl From<String> for IO {
    fn from(s: String) -> Self {

        Self(s.into())
    }
}

struct Ws {
    cons: Option<Addr<Cons>>,
    hb: Instant,
}

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Start heartbeat
        self.hb(ctx);

        // Start PTY
        self.cons = Some(Cons::new(ctx.address()).start());

        trace!("Started WebSocket");
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        trace!("Stopping WebSocket");

        Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        trace!("Stopped WebSocket");
    }
}

impl Handler<IO> for Ws {
    type Result = ();

    fn handle(&mut self, msg: IO, ctx: &mut <Self as Actor>::Context) {
        trace!("Ws <- Cons : {:?}", msg);
        ctx.binary(msg);
    }
}

impl Ws {
    pub fn new() -> Self {
        Self { hb: Instant::now(), cons: None }
    }

    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                warn!("Client heartbeat timeout, disconnecting.");
                ctx.stop();
                return;
            }

            ctx.ping("");
        });
    }
}

fn index(_req: &HttpRequest) -> Result<NamedFile> {
    Ok(NamedFile::open("static/term.html")?)
}

impl StreamHandler<ws::Message, ws::ProtocolError> for Ws {
    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        let cons: &mut Addr<Cons> = match self.cons {
            Some(ref mut c) => c,
            None => {
                error!("Console died, closing websocket.");
                ctx.stop();
                return;
            },
        };

        match msg {
            ws::Message::Ping(msg) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            },
            ws::Message::Pong(_) => self.hb = Instant::now(),
            ws::Message::Text(t) => cons.do_send(t.into()),
            ws::Message::Binary(b) => cons.do_send(b.into()),
            ws::Message::Close(_) => ctx.stop(),
        };
    }
}

struct Cons {
    pty_write: Option<WriteHalf<AsyncPtyMaster>>,
    child: Option<Child>,
    ws: Addr<Ws>,
}

impl Cons {
    pub fn new(ws: Addr<Ws>) -> Self {
        Self { pty_write: None, child: None, ws }
    }
}

impl StreamHandler<<BytesCodec as Decoder>::Item, <BytesCodec as Decoder>::Error> for Cons {
    fn handle(&mut self, msg: <BytesCodec as Decoder>::Item, _ctx: &mut Self::Context) {
        self.ws.do_send(IO(msg.into()));
    }
}

impl Actor for Cons {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Started Cons");
        let pty = match AsyncPtyMaster::open() {
            Err(e) => {
                error!("Unable to open PTY: {:?}", e);
                ctx.stop();
                return;
            }
            Ok(pty) => pty,
        };

        let child = match Command::new("/bin/sh").spawn_pty_async(&pty) {
            Err(e) => {
                error!("Unable to spawn child: {:?}", e);
                ctx.stop();
                return;
            }
            Ok(child) => child,
        };

        info!("Spawned new child process with PID {}", child.id());

        let (pty_read, pty_write) = pty.split();

        self.pty_write = Some(pty_write);
        self.child = Some(child);

        Self::add_stream(FramedRead::new(pty_read, BytesCodec::new()), ctx);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        info!("Stopping Cons");

        let child = self.child.take();

        if child.is_none() {
            // Great, child is already dead!
            return Running::Stop;
        }

        let mut child = child.unwrap();

        match child.kill() {
            Ok(()) => match child.wait() {
                Ok(exit) => info!("Child died: {:?}", exit),
                Err(e) => error!("Child wouldn't die: {}", e),
            },
            Err(e) => error!("Could not kill child with PID {}: {}", child.id(), e),
        };

        Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("Stopped Cons");
    }
}

impl Handler<IO> for Cons {
    type Result = ();

    fn handle(&mut self, msg: IO, ctx: &mut <Self as Actor>::Context) {
        let pty = match self.pty_write {
            Some(ref mut p) => p,
            None => {
                error!("Write half of PTY died, stopping Cons.");
                ctx.stop();
                return;
            }
        };

        pty.write(msg.as_ref());

        trace!("Ws -> Cons : {:?}", msg);
    }
}

fn main() {
    pretty_env_logger::init();

    server::new(|| {
        App::new()
            .handler(
                "/static",
                StaticFiles::new("node_modules")
                    .unwrap()
                    .show_files_listing(),
            )
            .resource("/websocket", |r| {
                r.f(|req| ws::start(req, Ws::new()))
            })
            .resource("/", |r| r.f(index))
    })
    .bind("127.0.0.1:8080")
    .unwrap()
    .run();
}
