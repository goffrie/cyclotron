use stdweb::unstable::TryInto;
use stdweb::web::html_element::CanvasElement;
use stdweb::UnsafeTypedArray;
use std::time::Duration;

use webgl_rendering_context::{GLenum, GLfloat, WebGLBuffer, WebGLProgram,
                              WebGLRenderingContext as GL, WebGLShader, WebGLTexture,
                              WebGLUniformLocation};

use spans;
use font;
use layout;

pub struct Options {
    pub start_ts: Duration,
    pub end_ts: Duration,
    pub font_size: u32,
}

#[derive(Default, Debug)]
pub struct Cache {
    font_cache: font::Cache,
    once: Option<Once>,
}

#[derive(Debug)]
struct Once {
    box_program: WebGLProgram,
    view_uniform: WebGLUniformLocation,
    color_uniform: WebGLUniformLocation,
    pos_buffer: WebGLBuffer,
    index_buffer: WebGLBuffer,
}

pub fn load_shader(gl: &GL, kind: GLenum, text: &str) -> WebGLShader {
    let shader = gl.create_shader(kind).unwrap();
    gl.shader_source(&shader, text);
    gl.compile_shader(&shader);
    let success: bool = gl.get_shader_parameter(&shader, GL::COMPILE_STATUS)
        .try_into()
        .unwrap();
    if !success {
        panic!(
            "failed to compile shader: {}",
            gl.get_shader_info_log(&shader).unwrap()
        );
    }
    shader
}

// apparently inlining this causes cargo-web to panic
fn info(canvas: &CanvasElement) -> (f64, u32, u32) {
    let size: Vec<f64> = js! {
        const canvas = @{canvas};
        const ratio = window.devicePixelRatio || 1;
        const width = Math.floor(canvas.clientWidth * ratio);
        const height = Math.floor(canvas.clientHeight * ratio);
        if (canvas.width != width || canvas.height != height) {
            canvas.width = width;
            canvas.height = height;
        }
        return [ratio, width, height];
    }.try_into()
        .unwrap();
    (size[0], size[1] as u32, size[2] as u32)
}

fn d(d: Duration) -> GLfloat {
    d.as_secs() as GLfloat + d.subsec_nanos() as GLfloat * 1e-9
}

fn render_boxes<'a, 'b: 'a>(
    gl: &GL,
    once: &Once,
    options: &Options,
    spans: impl Iterator<Item = &'a layout::LaidSpan<'b>>,
    col: (f32, f32, f32),
    pos_data: &mut Vec<GLfloat>,
    index_data: &mut Vec<u16>,
) {
    gl.use_program(Some(&once.box_program));
    gl.uniform4f(
        Some(&once.view_uniform),
        d(options.start_ts),
        0.0,
        d(options.end_ts),
        100.0,
    );
    gl.uniform3f(Some(&once.color_uniform), col.0, col.1, col.2);

    pos_data.clear();
    index_data.clear();

    for sp in spans {
        // two triangles make a rectangle
        let ix = (pos_data.len() / 2) as u16;
        index_data.push(ix);
        index_data.push(ix + 1);
        index_data.push(ix + 2);
        index_data.push(ix);
        index_data.push(ix + 2);
        index_data.push(ix + 3);

        let x1 = d(sp.span.start);
        let y1 = 2.0 * sp.row as GLfloat;
        let x2 = d(sp.span.end);
        let y2 = y1 + 1.5;
        pos_data.push(x1);
        pos_data.push(y1);
        pos_data.push(x1);
        pos_data.push(y2);
        pos_data.push(x2);
        pos_data.push(y2);
        pos_data.push(x2);
        pos_data.push(y1);
    }
    unsafe {
        let pos_data = UnsafeTypedArray::new(&pos_data);
        let index_data = UnsafeTypedArray::new(&index_data);
        js!{@(no_return)
            const gl = @{&gl};
            gl.bindBuffer(gl.ARRAY_BUFFER, @{&once.pos_buffer});
            gl.bufferData(gl.ARRAY_BUFFER, @{pos_data}, gl.DYNAMIC_DRAW);
            gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, @{&once.index_buffer});
            gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, @{index_data}, gl.DYNAMIC_DRAW);
        };
    }
    gl.draw_elements(
        GL::TRIANGLES,
        index_data.len() as i32,
        GL::UNSIGNED_SHORT,
        0,
    );
}

pub fn render(
    canvas: &CanvasElement,
    layout: &layout::Layout,
    options: &Options,
    cache: &mut Cache,
) {
    let (ratio, width, height) = info(canvas);
    let gl: GL = canvas.get_context().unwrap();
    if cache.once.is_none() {
        let box_frag = load_shader(&gl, GL::FRAGMENT_SHADER, include_str!("./shaders/box.frag"));
        let box_vert = load_shader(&gl, GL::VERTEX_SHADER, include_str!("./shaders/box.vert"));
        let box_program = gl.create_program().unwrap();
        gl.attach_shader(&box_program, &box_vert);
        gl.attach_shader(&box_program, &box_frag);
        gl.link_program(&box_program);
        let view_uniform = gl.get_uniform_location(&box_program, "view").unwrap();
        let color_uniform = gl.get_uniform_location(&box_program, "color").unwrap();

        macro_rules! mk_buffer {
            ($name: ident) => ({
                let buffer = gl.create_buffer().unwrap();
                gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buffer));
                let pos = gl.get_attrib_location(&box_program, stringify!($name)) as u32;
                gl.vertex_attrib_pointer(pos, 2, GL::FLOAT, false, 0, 0);
                gl.enable_vertex_attrib_array(pos);
                buffer
            })
        }
        let pos_buffer = mk_buffer!(pos);
        let index_buffer = gl.create_buffer().unwrap();
        let once = Once {
            box_program,
            view_uniform,
            color_uniform,
            pos_buffer,
            index_buffer,
        };
        cache.once = Some(once);
    }
    let font_size = (options.font_size as f64 * ratio) as u32;
    let once = cache.once.as_ref().unwrap();
    gl.viewport(0, 0, width as i32, height as i32);
    gl.clear_color(0.0, 0.0, 0.0, 0.0);
    gl.clear(GL::COLOR_BUFFER_BIT);

    // draw boxes
    let mut pos_data: Vec<GLfloat> = Vec::with_capacity(layout.spans.len() * 8);
    let mut index_data: Vec<u16> = Vec::with_capacity(layout.spans.len() * 6);

    for &(style, col) in &[
        (spans::SpanStyle::AsyncCancel, (0.3, 0.3, 0.7)),
        (spans::SpanStyle::AsyncError, (0.4, 0.1, 0.9)),
        (spans::SpanStyle::AsyncSuccess, (0.0, 0.0, 0.9)),
        (spans::SpanStyle::AsyncInProgress, (0.0, 0.0, 0.7)),
        (spans::SpanStyle::SyncFinished, (0.8, 0.8, 0.0)),
        (spans::SpanStyle::SyncInProgress, (0.6, 0.6, 0.0)),
        (spans::SpanStyle::ThreadFinished, (0.2, 0.8, 0.0)),
        (spans::SpanStyle::ThreadInProgress, (0.1, 0.7, 0.0)),
    ] {
        render_boxes(
            &gl,
            once,
            options,
            layout.spans.iter().filter(|sp| sp.span.style == style),
            col,
            &mut pos_data,
            &mut index_data,
        );
    }

    gl.enable(GL::BLEND);
    gl.blend_func(GL::ONE, GL::ONE_MINUS_SRC_ALPHA);

    font::draw_chars(&gl, &mut cache.font_cache, font_size, [
        (b'a', (0.0, 0.0)),
        (b'b', (80.0, 0.0)),
        (b'c', (160.0, 0.0)),
    ].iter().cloned(), (1.0, 0.5, 0.0));
}
