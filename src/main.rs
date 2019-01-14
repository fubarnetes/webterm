#![feature(try_from)]

extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate libc;
extern crate serde;
extern crate serde_json;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_io;
extern crate tokio_pty_process;
#[macro_use]
extern crate log;
extern crate pretty_env_logger;

use actix::*;
use actix_web::{fs::NamedFile, fs::StaticFiles, server, ws, App, Binary, HttpRequest, Result};

use futures::prelude::*;

use libc::c_ushort;

use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

use tokio_codec::{BytesCodec, Decoder, FramedRead};
use tokio_pty_process::{AsyncPtyMaster, AsyncPtyMasterWriteHalf, Child, CommandExt, PtyMaster};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

type BytesMut = <BytesCodec as Decoder>::Item;

mod terminado;
use crate::terminado::TerminadoMessage;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct IO(BytesMut);

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

impl From<&str> for IO {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

struct ChildDied();

impl Message for ChildDied {
    type Result = ();
}

struct Ws {
    req: HttpRequest,
    cons: Option<Addr<Cons>>,
    cmd_builder: fn(&HttpRequest) -> std::process::Command,
    hb: Instant,
}

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Start heartbeat
        self.hb(ctx);

        // Start PTY
        self.cons =
            Some(Cons::new(ctx.address(), self.cmd_builder.clone(), &self.req).start());

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

impl Handler<ChildDied> for Ws {
    type Result = ();

    fn handle(&mut self, msg: ChildDied, ctx: &mut <Self as Actor>::Context) {
        trace!("Ws got ChildDied Message");
        ctx.close(None);
        ctx.stop();
    }
}

impl Handler<TerminadoMessage> for Ws {
    type Result = ();

    fn handle(&mut self, msg: TerminadoMessage, ctx: &mut <Self as Actor>::Context) {
        trace!("Ws <- Cons : {:?}", msg);
        match msg {
            TerminadoMessage::Stdout(_) => {
                let json = serde_json::to_string(&msg);

                if let Ok(json) = json {
                    ctx.text(json);
                }
            }
            _ => error!(r#"Invalid TerminadoMessage to Websocket: only "stdout" supported"#),
        }
    }
}

impl Ws {
    pub fn new(cmd_builder: fn(&HttpRequest) -> std::process::Command, req: &HttpRequest) -> Self {
        Self {
            req: req.to_owned(),
            hb: Instant::now(),
            cmd_builder,
            cons: None,
        }
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
            }
        };

        match msg {
            ws::Message::Ping(msg) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            ws::Message::Pong(_) => self.hb = Instant::now(),
            ws::Message::Text(t) => {
                // Attempt to parse the message as JSON.
                if let Ok(tmsg) = TerminadoMessage::from_json(&t) {
                    cons.do_send(tmsg);
                } else {
                    // Otherwise, it's just byte data.
                    cons.do_send(IO::from(t));
                }
            }
            ws::Message::Binary(b) => cons.do_send(IO::from(b)),
            ws::Message::Close(_) => ctx.stop(),
        };
    }
}

struct Cons {
    cmd_builder: fn(&HttpRequest) -> std::process::Command,
    req: HttpRequest,
    pty_write: Option<AsyncPtyMasterWriteHalf>,
    child: Option<Child>,
    ws: Addr<Ws>,
}

impl Cons {
    pub fn new(
        ws: Addr<Ws>,
        cmd_builder: fn(&HttpRequest) -> std::process::Command,
        req: &HttpRequest,
    ) -> Self {
        Self {
            cmd_builder,
            req: req.clone(),
            pty_write: None,
            child: None,
            ws,
        }
    }
}

impl StreamHandler<<BytesCodec as Decoder>::Item, <BytesCodec as Decoder>::Error> for Cons {
    fn handle(&mut self, msg: <BytesCodec as Decoder>::Item, _ctx: &mut Self::Context) {
        self.ws.do_send(TerminadoMessage::Stdout(IO(msg)));
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
        let cmd_builder = self.cmd_builder;
        let child = match cmd_builder(&self.req).spawn_pty_async(&pty) {
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

        self.ws.do_send(ChildDied());
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

struct Resize<T: PtyMaster> {
    pty: T,
    rows: c_ushort,
    cols: c_ushort,
}

impl<T: PtyMaster> Future for Resize<T> {
    type Item = ();
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.pty.resize(self.rows, self.cols)
    }
}

impl Handler<TerminadoMessage> for Cons {
    type Result = ();

    fn handle(&mut self, msg: TerminadoMessage, ctx: &mut <Self as Actor>::Context) {
        let pty = match self.pty_write {
            Some(ref mut p) => p,
            None => {
                error!("Write half of PTY died, stopping Cons.");
                ctx.stop();
                return;
            }
        };

        trace!("Ws -> Cons : {:?}", msg);
        match msg {
            TerminadoMessage::Stdin(io) => {
                pty.write(io.as_ref());
            }
            TerminadoMessage::Resize { cols, rows } => {
                info!("Resize: cols = {}, rows = {}", cols, rows);
                Resize { pty, cols, rows }.wait().map_err(|e| {
                    error!("Resize failed: {}", e);
                });
            }
            TerminadoMessage::Stdout(_) => {
                error!("Invalid Terminado Message: Stdin cannot go to PTY")
            }
        };
    }
}

fn run(addr: &str, cmd_builder: fn(&HttpRequest) -> std::process::Command) {
    server::new(move || {
        App::new()
            .handler(
                "/static",
                StaticFiles::new("node_modules")
                    .unwrap()
                    .show_files_listing(),
            )
            .resource("/websocket", move |r| {
                r.f(move |req| ws::start(req, Ws::new(cmd_builder, req)))
            })
            .resource("/", |r| r.f(index))
    })
    .bind(addr)
    .unwrap()
    .run();
}

fn main() {
    pretty_env_logger::init();
    run("127.0.0.1:8080", |req| {
        let mut builder = std::process::Command::new("/bin/sh");
        builder.env("URL", req.uri().to_string());
        builder
    });
}
