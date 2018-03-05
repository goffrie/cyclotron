use stdweb::unstable::TryInto;
use stdweb::web::html_element::CanvasElement;
use stdweb::UnsafeTypedArray;
use std::time::Duration;
use font_rs::font;

use webgl_rendering_context::{WebGLBuffer, WebGLRenderingContext as GL, WebGLTexture,
                              WebGLUniformLocation, WebGLShader, GLenum, WebGLProgram, GLfloat};

use render::load_shader;

const FONT: &[u8] = include_bytes!("./Inconsolata-Regular.ttf");

#[derive(Debug)]
struct Once {
    program: WebGLProgram,
    view_uniform: WebGLUniformLocation,
    atlas_uniform: WebGLUniformLocation,
    col_uniform: WebGLUniformLocation,
    pos_buffer: WebGLBuffer,
    tex_coord_buffer: WebGLBuffer,
    index_buffer: WebGLBuffer,
}

#[derive(Debug, Default)]
pub struct Cache {
    once: Option<Once>,
    atlas: Option<(u32, (WebGLTexture, u32, u32))>,
}

fn gen_atlas(gl: &GL, font_size: u32) -> (WebGLTexture, u32, u32) {
    let font = font::parse(FONT).unwrap();
    let glyphs: Vec<_> = (32..127)
        .filter_map(|code| {
            font.render_glyph(code, font_size)
                .map(|glyph| (code, glyph))
        })
        .collect();
    let width = glyphs
        .iter()
        .map(|&(_, ref glyph)| glyph.width)
        .max()
        .unwrap() as usize;
    let height = glyphs
        .iter()
        .map(|&(_, ref glyph)| glyph.height)
        .max()
        .unwrap() as usize;
    // XXX
    let xx = 16;
    let yy = 8;
    let atlas_width = xx * width;
    let atlas_height = yy * height;
    let mut atlas = vec![0u8; atlas_width * atlas_height];
    for (code, glyph) in glyphs {
        let x = code as usize % xx;
        let y = code as usize / xx;
        for gy in 0..(glyph.height as usize) {
            for gx in 0..(glyph.width as usize) {
                atlas[(y * height + gy) * atlas_width + (x * width + gx)] =
                    glyph.data[gy * glyph.width + gx];
            }
        }
    }
    let tex = gl.create_texture().unwrap();
    unsafe {
        let atlas = UnsafeTypedArray::new(&atlas);
        js!{@(no_return)
            const gl = @{gl};
            gl.bindTexture(gl.TEXTURE_2D, @{&tex});
            gl.texImage2D(
                gl.TEXTURE_2D,
                0,
                gl.LUMINANCE,
                @{atlas_width as f64}, @{atlas_height as f64},
                0,
                gl.LUMINANCE,
                gl.UNSIGNED_BYTE,
                @{&atlas}
            );
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        };
    }
    (tex, width as u32, height as u32)
}

fn warm(gl: &GL, cache: &mut Cache, font_size: u32) {
    if cache.once.is_none() {
        let frag = load_shader(&gl, GL::FRAGMENT_SHADER, include_str!("./shaders/glyph.frag"));
        let vert = load_shader(&gl, GL::VERTEX_SHADER, include_str!("./shaders/glyph.vert"));
        let program = gl.create_program().unwrap();
        gl.attach_shader(&program, &vert);
        gl.attach_shader(&program, &frag);
        gl.link_program(&program);
        let view_uniform = gl.get_uniform_location(&program, "view").unwrap();
        let col_uniform = gl.get_uniform_location(&program, "color").unwrap();
        let atlas_uniform = gl.get_uniform_location(&program, "atlas").unwrap();

        macro_rules! mk_buffer {
            ($name: ident) => ({
                let buffer = gl.create_buffer().unwrap();
                gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buffer));
                let pos = gl.get_attrib_location(&program, stringify!($name)) as u32;
                gl.vertex_attrib_pointer(pos, 2, GL::FLOAT, false, 0, 0);
                gl.enable_vertex_attrib_array(pos);
                buffer
            })
        }
        let pos_buffer = mk_buffer!(pos);
        let tex_coord_buffer = mk_buffer!(tex_coord);
        let index_buffer = gl.create_buffer().unwrap();
        let once = Once {
            program,
            view_uniform,
            col_uniform,
            atlas_uniform,
            pos_buffer,
            tex_coord_buffer,
            index_buffer,
        };
        cache.once = Some(once);
    }
    let once = cache.once.as_ref().unwrap();
    match cache.atlas {
        Some((cached_size, _)) if cached_size == font_size => {}
        _ => cache.atlas = Some((font_size, gen_atlas(&gl, font_size))),
    }
}

pub fn glyph_size(gl: &GL, cache: &mut Cache, font_size: u32) -> (u32, u32) {
    warm(gl, cache, font_size);
    let (_, w, h) = cache.atlas.as_ref().unwrap().1;
    (w, h)
}

pub fn draw_chars(gl: &GL, cache: &mut Cache, font_size: u32, chars: impl Iterator<Item = (u8, (f32, f32))>, color: (f32, f32, f32)) {
    warm(gl, cache, font_size);
    let cw = gl.canvas().width();
    let ch = gl.canvas().height();
    let once = cache.once.as_ref().unwrap();
    let (ref atlas, w, h) = cache.atlas.as_ref().unwrap().1;
    gl.use_program(Some(&once.program));
    gl.active_texture(GL::TEXTURE0);
    gl.bind_texture(GL::TEXTURE_2D, Some(&atlas));
    gl.uniform1i(Some(&once.atlas_uniform), 0);
    gl.uniform3f(Some(&once.col_uniform), color.0, color.1, color.2);
    gl.uniform4f(Some(&once.view_uniform), 0.0, 0.0, cw as f32, ch as f32);

    let mut pos_data = vec![];
    let mut tex_coord_data = vec![];
    let mut index_data = vec![];

    for (ch, (x1, y1)) in chars {
        let ix = (pos_data.len() / 2) as u16;
        index_data.push(ix);
        index_data.push(ix + 1);
        index_data.push(ix + 2);
        index_data.push(ix);
        index_data.push(ix + 2);
        index_data.push(ix + 3);

        let x2 = x1 + w as f32;
        let y2 = y1 + h as f32;
        pos_data.push(x1);
        pos_data.push(y1);
        pos_data.push(x1);
        pos_data.push(y2);
        pos_data.push(x2);
        pos_data.push(y2);
        pos_data.push(x2);
        pos_data.push(y1);

        // copy pasta
        let xx = 16;
        let yy = 8;
        let x = ch as usize % xx;
        let y = ch as usize / xx;

        let tx1 = x as f32 / xx as f32;
        let ty1 = y as f32 / yy as f32;
        let tx2 = (x + 1) as f32 / xx as f32;
        let ty2 = (y + 1) as f32 / yy as f32;
        tex_coord_data.push(tx1);
        tex_coord_data.push(ty1);
        tex_coord_data.push(tx1);
        tex_coord_data.push(ty2);
        tex_coord_data.push(tx2);
        tex_coord_data.push(ty2);
        tex_coord_data.push(tx2);
        tex_coord_data.push(ty1);
    }
    unsafe {
        let pos_data = UnsafeTypedArray::new(&pos_data);
        let tex_coord_data = UnsafeTypedArray::new(&tex_coord_data);
        let index_data = UnsafeTypedArray::new(&index_data);
        js!{@(no_return)
            const gl = @{&gl};
            gl.bindBuffer(gl.ARRAY_BUFFER, @{&once.pos_buffer});
            gl.bufferData(gl.ARRAY_BUFFER, @{pos_data}, gl.DYNAMIC_DRAW);
            gl.bindBuffer(gl.ARRAY_BUFFER, @{&once.tex_coord_buffer});
            gl.bufferData(gl.ARRAY_BUFFER, @{tex_coord_data}, gl.DYNAMIC_DRAW);
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
