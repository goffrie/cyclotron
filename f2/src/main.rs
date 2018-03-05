#![recursion_limit = "128"]
#![feature(proc_macro)]
#![feature(conservative_impl_trait)]
#![feature(universal_impl_trait)]

extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate stdweb;
#[macro_use]
extern crate stdweb_derive;
extern crate smallvec;

extern crate font_rs;

pub mod webgl_rendering_context;

mod event;
mod spans;
mod render;
mod font;
mod layout;

use std::rc::Rc;
use std::cell::{Cell, RefCell};
use std::time::Duration;
use stdweb::web::html_element::CanvasElement;
use stdweb::web::*;
use stdweb::web::event::{ChangeEvent, ResizeEvent};
use stdweb::traits::*;
use stdweb::{Array, InstanceOf, Reference, Undefined, Value};
use stdweb::unstable::{TryFrom, TryInto};

use serde::Deserialize;

macro_rules! enclose {
    ( ($( $x:ident ),*) $y:expr ) => {
        {
            $(let $x = $x.clone();)*
            $y
        }
    };
}

pub struct Inner {
    canvas: CanvasElement,
    spans: RefCell<spans::State>,
    render_cache: RefCell<render::Cache>,
    zoom: Cell<(Duration, Duration)>,
    render_scheduled: Cell<bool>,
}

#[derive(Clone)]
pub struct Context {
    inner: Rc<Inner>,
}

fn read_into(out: &mut spans::State, bytes: &[u8]) -> Result<(), serde_json::Error> {
    for item in serde_json::StreamDeserializer::new(serde_json::de::SliceRead::new(bytes)) {
        out.add_event(item?);
    }
    Ok(())
}

impl Context {
    fn render(&self, _time: f64) {
        let (start, end) = self.inner.zoom.get();
        let state = self.inner.spans.borrow();
        let layout = layout::lay_out(state.select(start, end));
        render::render(&self.inner.canvas, &layout, &render::Options {
            start_ts: start,
            end_ts: end,
            font_size: 120,
        }, &mut self.inner.render_cache.borrow_mut());

        self.schedule_render();
    }

    fn schedule_render(&self) {
        let ctx = self.clone();
        if !self.inner.render_scheduled.get() {
            self.inner.render_scheduled.set(true);
            window().request_animation_frame(move |time| {
                ctx.inner.render_scheduled.set(false);
                ctx.render(time);
            });
        }
    }

    fn set_file(&self, file: Reference) {
        let this = self.clone();
        let callback = move |array: ArrayBuffer| {
            let data: Vec<u8> = array.into();
            {
                let mut spans = this.inner.spans.borrow_mut();
                *spans = spans::State::new();
                if let Err(e) = read_into(&mut spans, &data) {
                    console!(error, format!("JSON deserialization error: {}", e));
                }
                console!(log, format!("Loaded in {} spans", spans.len()));
                this.inner.zoom.set((Duration::default(), spans.end_time));
            }
            this.schedule_render();
        };
        js!{@(no_return)
            const reader = new FileReader();
            reader.addEventListener("loadend", function() {
                (@{callback})(reader.result);
            });
            reader.readAsArrayBuffer(@{&file});
        }
    }
}

pub fn main() {
    stdweb::initialize();
    let canvas: CanvasElement = document()
        .get_element_by_id("canvas")
        .unwrap()
        .try_into()
        .unwrap();
    let ctx = Context {
        inner: Rc::new(Inner {
            canvas,
            zoom: Default::default(),
            spans: RefCell::new(spans::State::new()),
            render_cache: Default::default(),
            render_scheduled: Cell::new(false),
        }),
    };
    window().add_event_listener(enclose!((ctx) move |_e: ResizeEvent| {
        ctx.schedule_render();
    }));
    document()
        .get_element_by_id("file")
        .unwrap()
        .add_event_listener(enclose!((ctx) move |e: ChangeEvent| {
            let file = js! {
                return document.getElementById("file").files[0];
            };
            if let Value::Reference(file) = file {
                ctx.set_file(file);
            }
            e.prevent_default();
        }));
    ctx.schedule_render();
    stdweb::event_loop();
}
