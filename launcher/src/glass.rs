use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};

#[derive(Clone, Copy)]
pub struct Look {
    pub radius: f32,
    pub tint: Color32,
    pub amount: f32,
    pub refract: f32,
    pub blur: f32,
    pub zoom: f32,
    pub rim: f32,
    pub shadow: f32,
    pub glow: f32,
    pub color: Color32,
    pub warm: f32,
}

impl Look {
    pub fn clear(radius: f32) -> Self {
        Look {
            radius,
            tint: Color32::WHITE,
            amount: 0.04,
            refract: 5.0,
            blur: 1.2,
            zoom: 1.0,
            rim: 1.0,
            shadow: 0.3,
            glow: 1.0,
            color: Color32::WHITE,
            warm: 0.0,
        }
    }
}

struct Res {
    program: glow::Program,
    vao: glow::VertexArray,
    tex: glow::Texture,
    u: [Option<glow::UniformLocation>; 10],
}

enum State {
    Fresh,
    Ready(Res),
    Broken,
}

static STATE: Mutex<State> = Mutex::new(State::Fresh);
static BROKEN: AtomicBool = AtomicBool::new(false);

const UNIFORMS: [&str; 10] = ["u_tex", "u_copy", "u_center", "u_half", "u_radius", "u_ppp", "u_tint", "u_fx", "u_fx2", "u_warm"];

const VERTEX: &str = r#"#version 140
const vec2 Q[4] = vec2[4](vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, 1.0));
void main() { gl_Position = vec4(Q[gl_VertexID], 0.0, 1.0); }
"#;

const FRAGMENT: &str = r#"#version 140
uniform sampler2D u_tex;
uniform vec4 u_copy;
uniform vec2 u_center;
uniform vec2 u_half;
uniform float u_radius;
uniform float u_ppp;
uniform vec4 u_tint;
uniform vec4 u_fx;
uniform vec4 u_fx2;
uniform vec4 u_warm;
out vec4 o;

float sd(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

vec2 nrm(vec2 p, vec2 b, float r) {
    float e = 0.5;
    vec2 g = vec2(sd(p + vec2(e, 0.0), b, r) - sd(p - vec2(e, 0.0), b, r),
                  sd(p + vec2(0.0, e), b, r) - sd(p - vec2(0.0, e), b, r));
    return g / max(length(g), 1e-5);
}

vec4 at(vec2 px) {
    return texture(u_tex, (px - u_copy.xy) / u_copy.zw);
}

vec4 soft(vec2 px, float r) {
    if (r < 0.5) return at(px);
    vec4 s = at(px) * 0.2;
    for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.7853982;
        s += at(px + vec2(cos(a), sin(a)) * r) * 0.1;
    }
    return s;
}

void main() {
    vec2 p = gl_FragCoord.xy - u_center;
    float r = min(u_radius, min(u_half.x, u_half.y));
    float d = sd(p, u_half, r);
    float ds = sd(p + vec2(0.0, 3.0 * u_ppp), u_half, r);
    float sh = u_fx2.x * (1.0 - smoothstep(-3.0 * u_ppp, 10.0 * u_ppp, ds));
    float cov = clamp(0.5 - d, 0.0, 1.0);
    if (cov <= 0.0 && sh <= 0.002) discard;

    vec2 n = nrm(p, u_half, r);
    float bezel = min(min(u_half.x, u_half.y), 14.0 * u_ppp);
    float t = clamp(-d / bezel, 0.0, 1.0);
    float edge = 1.0 - t;
    float bend = edge * edge * edge;
    vec2 base = u_center + p * u_fx.z;
    vec2 off = n * u_fx.x * bend;

    vec4 mid = soft(base + off, u_fx.y);
    vec3 col = vec3(soft(base + off * 1.25, u_fx.y).r, mid.g, soft(base + off * 0.75, u_fx.y).b);
    float a = mid.a;

    col = col * (1.0 - u_tint.a) + u_tint.rgb * u_tint.a;

    vec2 corner = u_center + vec2(-u_half.x, u_half.y);
    float reach = max(u_half.x * 1.25, u_half.y * 3.0);
    float g = 1.0 - clamp(length(gl_FragCoord.xy - corner) / reach, 0.0, 1.0);
    g = g * g * (3.0 - 2.0 * g);
    col = mix(col, u_warm.rgb, g * u_warm.a);

    vec2 L = normalize(vec2(-0.55, 0.85));
    float facing = dot(n, L);
    float band = 1.0 - smoothstep(0.0, 1.5 * u_ppp, -d);
    float spec = pow(abs(facing), 3.0);
    float rim = u_fx.w * band * (0.10 + 0.70 * spec) * (facing > 0.0 ? 1.0 : 0.6);
    float inner = u_fx2.y * pow(edge, 4.0) * max(facing, 0.0) * 0.22;
    float under = u_fx2.y * pow(edge, 4.0) * max(-facing, 0.0) * 0.10;
    vec3 light = mix(vec3(1.0), u_warm.rgb, clamp(g * u_warm.a * 1.6, 0.0, 0.8));
    col += light * (rim + inner);
    col *= 1.0 - under;

    o = vec4(col * cov, cov + sh * (1.0 - cov));
}
"#;

pub fn ok() -> bool {
    !BROKEN.load(Ordering::Relaxed)
}

pub fn paint(painter: &egui::Painter, rect: Rect, look: Look) {
    painter.add(shape(rect, look));
}

pub fn shape(rect: Rect, look: Look) -> egui::Shape {
    let area = rect.expand(14.0);
    let callback = egui_glow::CallbackFn::new(move |info, p| unsafe { draw(p.gl(), &info, rect, &look) });
    egui::Shape::Callback(egui::PaintCallback { rect: area, callback: Arc::new(callback) })
}

unsafe fn compile(gl: &glow::Context, kind: u32, src: &str) -> Result<glow::Shader, String> {
    unsafe {
        let s = gl.create_shader(kind)?;
        gl.shader_source(s, src);
        gl.compile_shader(s);
        if gl.get_shader_compile_status(s) {
            Ok(s)
        } else {
            let log = gl.get_shader_info_log(s);
            gl.delete_shader(s);
            Err(log)
        }
    }
}

unsafe fn build(gl: &glow::Context) -> Result<Res, String> {
    unsafe {
        let vs = compile(gl, glow::VERTEX_SHADER, VERTEX)?;
        let fs = compile(gl, glow::FRAGMENT_SHADER, FRAGMENT)?;
        let program = gl.create_program()?;
        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);
        gl.detach_shader(program, vs);
        gl.detach_shader(program, fs);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(log);
        }
        let vao = gl.create_vertex_array()?;
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        let u = UNIFORMS.map(|name| gl.get_uniform_location(program, name));
        Ok(Res { program, vao, tex, u })
    }
}

unsafe fn draw(gl: &glow::Context, info: &egui::PaintCallbackInfo, rect: Rect, look: &Look) {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(*state, State::Fresh) {
        *state = match unsafe { build(gl) } {
            Ok(res) => State::Ready(res),
            Err(_) => {
                BROKEN.store(true, Ordering::Relaxed);
                State::Broken
            }
        };
    }
    let State::Ready(res) = &*state else { return };
    let ppp = info.pixels_per_point;
    let [sw, sh] = info.screen_size_px;
    let v = info.viewport_in_pixels();
    let x0 = v.left_px.max(0);
    let y0 = v.from_bottom_px.max(0);
    let x1 = (v.left_px + v.width_px).min(sw as i32);
    let y1 = (v.from_bottom_px + v.height_px).min(sh as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let c = |v: u8| v as f32 / 255.0;
    let u = &res.u;
    unsafe {
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(res.tex));
        gl.copy_tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA, x0, y0, x1 - x0, y1 - y0, 0);
        gl.use_program(Some(res.program));
        gl.uniform_1_i32(u[0].as_ref(), 0);
        gl.uniform_4_f32(u[1].as_ref(), x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32);
        gl.uniform_2_f32(u[2].as_ref(), rect.center().x * ppp, sh as f32 - rect.center().y * ppp);
        gl.uniform_2_f32(u[3].as_ref(), rect.width() * 0.5 * ppp, rect.height() * 0.5 * ppp);
        gl.uniform_1_f32(u[4].as_ref(), look.radius * ppp);
        gl.uniform_1_f32(u[5].as_ref(), ppp);
        gl.uniform_4_f32(u[6].as_ref(), c(look.tint.r()), c(look.tint.g()), c(look.tint.b()), look.amount.clamp(0.0, 1.0));
        gl.uniform_4_f32(u[7].as_ref(), look.refract * ppp, look.blur * ppp, look.zoom, look.rim);
        gl.uniform_4_f32(u[8].as_ref(), look.shadow, look.glow, 0.0, 0.0);
        gl.uniform_4_f32(u[9].as_ref(), c(look.color.r()), c(look.color.g()), c(look.color.b()), look.warm.clamp(0.0, 1.0));
        gl.color_mask(true, true, true, false);
        gl.bind_vertex_array(Some(res.vao));
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        gl.bind_vertex_array(None);
    }
}
