// Copyright (c) 2019 Fabian Freyer <fabian.freyer@physik.tu-berlin.de>.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice,
//    this list of conditions and the following disclaimer.
//
// 2: Redistributions in binary form must reproduce the above copyright notice,
//    this list of conditions and the following disclaimer in the documentation
//    and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors
//    may be used to endorse or promote products derived from this software
//    without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.

extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate handlebars;
extern crate libc;
extern crate serde;
#[macro_use]
extern crate serde_json;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_io;
extern crate tokio_pty_process;
#[macro_use]
extern crate log;
extern crate pretty_env_logger;

use actix::prelude::*;
use actix::{Actor, StreamHandler};
use actix_web::{web, App, HttpRequest, HttpResponse};
use actix_web_actors::ws;

use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

use tokio_codec::{BytesCodec, Decoder, FramedRead};
use tokio_pty_process::{AsyncPtyMaster, AsyncPtyMasterWriteHalf, Child, CommandExt};

use handlebars::Handlebars;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

mod event;
mod terminado;

/// Actix WebSocket actor
pub struct Websocket {
    cons: Option<Addr<Terminal>>,
    hb: Instant,
    command: Option<Command>,
}

impl Actor for Websocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Start heartbeat
        self.hb(ctx);

        let command = self
            .command
            .take()
            .expect("command was None at start of WebSocket.");

        // Start PTY
        self.cons = Some(Terminal::new(ctx.address(), command).start());

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

impl Handler<event::IO> for Websocket {
    type Result = ();

    fn handle(&mut self, msg: event::IO, ctx: &mut <Self as Actor>::Context) {
        trace!("Websocket <- Terminal : {:?}", msg);
        ctx.binary(msg);
    }
}

impl Handler<event::TerminadoMessage> for Websocket {
    type Result = ();

    fn handle(&mut self, msg: event::TerminadoMessage, ctx: &mut <Self as Actor>::Context) {
        trace!("Websocket <- Terminal : {:?}", msg);
        match msg {
            event::TerminadoMessage::Stdout(_) => {
                let json = serde_json::to_string(&msg);

                if let Ok(json) = json {
                    ctx.text(json);
                }
            }
            _ => error!(r#"Invalid event::TerminadoMessage to Websocket: only "stdout" supported"#),
        }
    }
}

impl Websocket {
    pub fn new(command: Command) -> Self {
        Self {
            hb: Instant::now(),
            cons: None,
            command: Some(command),
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

impl StreamHandler<ws::Message, ws::ProtocolError> for Websocket {
    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        let cons: &mut Addr<Terminal> = match self.cons {
            Some(ref mut c) => c,
            None => {
                error!("Terminalole died, closing websocket.");
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
                if let Ok(tmsg) = event::TerminadoMessage::from_json(&t) {
                    cons.do_send(tmsg);
                } else {
                    // Otherwise, it's just byte data.
                    cons.do_send(event::IO::from(t));
                }
            }
            ws::Message::Binary(b) => cons.do_send(event::IO::from(b)),
            ws::Message::Close(_) => ctx.stop(),
            ws::Message::Nop => {}
        };
    }
}

impl Handler<event::ChildDied> for Websocket {
    type Result = ();

    fn handle(&mut self, _msg: event::ChildDied, ctx: &mut <Self as Actor>::Context) {
        trace!("Websocket <- ChildDied");
        ctx.close(None);
        ctx.stop();
    }
}

/// Represents a PTY backenActix WebSocket actor.d with attached child
pub struct Terminal {
    pty_write: Option<AsyncPtyMasterWriteHalf>,
    child: Option<Child>,
    ws: Addr<Websocket>,
    command: Command,
}

impl Terminal {
    pub fn new(ws: Addr<Websocket>, command: Command) -> Self {
        Self {
            pty_write: None,
            child: None,
            ws,
            command,
        }
    }
}

impl StreamHandler<<BytesCodec as Decoder>::Item, <BytesCodec as Decoder>::Error> for Terminal {
    fn handle(&mut self, msg: <BytesCodec as Decoder>::Item, _ctx: &mut Self::Context) {
        self.ws
            .do_send(event::TerminadoMessage::Stdout(event::IO(msg)));
    }
}

impl Actor for Terminal {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Started Terminal");
        let pty = match AsyncPtyMaster::open() {
            Err(e) => {
                error!("Unable to open PTY: {:?}", e);
                ctx.stop();
                return;
            }
            Ok(pty) => pty,
        };

        let child = match self.command.spawn_pty_async(&pty) {
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
        info!("Stopping Terminal");

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

        // Notify the websocket that the child died.
        self.ws.do_send(event::ChildDied());

        Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("Stopped Terminal");
    }
}

impl Handler<event::IO> for Terminal {
    type Result = ();

    fn handle(&mut self, msg: event::IO, ctx: &mut <Self as Actor>::Context) {
        let pty = match self.pty_write {
            Some(ref mut p) => p,
            None => {
                error!("Write half of PTY died, stopping Terminal.");
                ctx.stop();
                return;
            }
        };

        if let Err(e) = pty.write(msg.as_ref()) {
            error!("Could not write to PTY: {}", e);
            ctx.stop();
        }

        trace!("Websocket -> Terminal : {:?}", msg);
    }
}

impl Handler<event::TerminadoMessage> for Terminal {
    type Result = ();

    fn handle(&mut self, msg: event::TerminadoMessage, ctx: &mut <Self as Actor>::Context) {
        let pty = match self.pty_write {
            Some(ref mut p) => p,
            None => {
                error!("Write half of PTY died, stopping Terminal.");
                ctx.stop();
                return;
            }
        };

        trace!("Websocket -> Terminal : {:?}", msg);
        match msg {
            event::TerminadoMessage::Stdin(io) => {
                if let Err(e) = pty.write(io.as_ref()) {
                    error!("Could not write to PTY: {}", e);
                    ctx.stop();
                }
            }
            event::TerminadoMessage::Resize { rows, cols } => {
                info!("Resize: cols = {}, rows = {}", cols, rows);
                if let Err(e) = event::Resize::new(pty, rows, cols).wait() {
                    error!("Resize failed: {}", e);
                    ctx.stop();
                }
            }
            event::TerminadoMessage::Stdout(_) => {
                error!("Invalid Terminado Message: Stdin cannot go to PTY")
            }
        };
    }
}

/// Trait to extend an [actix_web::App] by serving a web terminal.
pub trait WebTermExt {
    /// Serve the websocket for the webterm
    fn webterm_socket<F>(self: Self, endpoint: &str, handler: F) -> Self
    where
        F: Clone + Fn(&actix_web::HttpRequest) -> Command + 'static;

    fn webterm_ui(
        self: Self,
        endpoint: &str,
        webterm_socket_endpoint: &str,
        static_path: &str,
    ) -> Self;
}

impl<T, B> WebTermExt for App<T, B>
where
    B: actix_web::body::MessageBody,
    T: actix_service::NewService<
        Config = (),
        Request = actix_web::dev::ServiceRequest,
        Response = actix_web::dev::ServiceResponse<B>,
        Error = actix_web::Error,
        InitError = (),
    >,
{
    fn webterm_socket<F>(self: Self, endpoint: &str, handler: F) -> Self
    where
        F: Clone + Fn(&actix_web::HttpRequest) -> Command + 'static,
    {
        self.route(
            endpoint,
            web::get().to(move |req: HttpRequest, stream: web::Payload| {
                ws::start(Websocket::new(handler(&req)), &req, stream)
            }),
        )
    }

    fn webterm_ui(
        self: Self,
        endpoint: &str,
        webterm_socket_endpoint: &str,
        static_path: &str,
    ) -> Self {
        let mut handlebars = Handlebars::new();
        handlebars
            .register_templates_directory(".html", "./templates")
            .unwrap();
        let handlebars_ref = web::Data::new(handlebars);
        let static_path = static_path.to_owned();
        let webterm_socket_endpoint = webterm_socket_endpoint.to_owned();
        self.register_data(handlebars_ref.clone()).route(
            endpoint,
            web::get().to(move |hb: web::Data<Handlebars>| {
                let data = json!({
                "websocket_path": webterm_socket_endpoint,
                "static_path": static_path,
                });
                let body = hb.render("term", &data).unwrap();
                HttpResponse::Ok().body(body)
            }),
        )
    }
}
