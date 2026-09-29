use crate::board3d::{AMBIENT, BACKGROUND, Camera, FILL, KEY, Scene, view};
use eframe::egui_glow;
use egui::{Rect, Ui};
use std::cell::RefCell;
use std::sync::Arc;
use three_d::*;

struct Shaded {
    colour: Vec3,
    texture: Option<Arc<Texture2D>>,
    metal: f32,
    eye: Vec3,
    key: Vec3,
    fill: Vec3,
}

impl Material for Shaded {
    fn id(&self) -> EffectMaterialId {
        EffectMaterialId(0x9100 | self.texture.is_some() as u16)
    }

    fn fragment_shader_source(&self, _lights: &[&dyn Light]) -> String {
        let mut s = String::new();
        if self.texture.is_some() {
            s.push_str("#define USE_TEXTURE\nin vec2 uvs;\nuniform sampler2D tex;\n");
        }
        s.push_str(&format!(
            "const float AMBIENT = {AMBIENT:.4};\nconst float KEY = {KEY:.4};\nconst float FILL = {FILL:.4};\n"
        ));
        s.push_str(
            r#"
uniform vec3 surfaceColour;
uniform float metal;
uniform vec3 eye;
uniform vec3 keyDir;
uniform vec3 fillDir;
in vec3 pos;
in vec3 nor;
layout (location = 0) out vec4 outColor;

vec3 to_linear(vec3 c) {
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(vec3(0.04045), c));
}

vec3 to_srgb(vec3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(vec3(0.0031308), c));
}

void main() {
    vec3 albedo = surfaceColour;
#ifdef USE_TEXTURE
    albedo = texture(tex, uvs).rgb;
#endif
    albedo = to_linear(albedo);
    vec3 n = normalize(nor);
    vec3 v = normalize(eye - pos);
    if (dot(n, v) < 0.0) n = -n;
    float d = AMBIENT + KEY * max(dot(n, keyDir), 0.0) + FILL * max(dot(n, fillDir), 0.0);
    vec3 h = normalize(keyDir + v);
    float ks = 0.06 + 0.5 * metal;
    float sh = 24.0 + 72.0 * metal;
    float sp = ks * pow(max(dot(n, h), 0.0), sh);
    vec3 tint = vec3(1.0) + (albedo * 1.6 - vec3(1.0)) * metal;
    outColor = vec4(to_srgb(albedo * d + sp * tint), 1.0);
}
"#,
        );
        s
    }

    fn use_uniforms(&self, program: &Program, _viewer: &dyn Viewer, _lights: &[&dyn Light]) {
        program.use_uniform_if_required("surfaceColour", self.colour);
        program.use_uniform_if_required("metal", self.metal);
        program.use_uniform_if_required("eye", self.eye);
        program.use_uniform_if_required("keyDir", self.key);
        program.use_uniform_if_required("fillDir", self.fill);
        if let Some(t) = &self.texture {
            program.use_texture("tex", t);
        }
    }

    fn render_states(&self) -> RenderStates {
        RenderStates { cull: Cull::None, ..Default::default() }
    }

    fn material_type(&self) -> MaterialType {
        MaterialType::Opaque
    }
}

struct Gpu {
    context: Context,
    scene: u64,
    objects: Vec<(bool, Gm<Mesh, Shaded>)>,
}

impl Gpu {
    fn upload(&mut self, scene: &Scene) {
        let textures: Vec<Arc<Texture2D>> = scene
            .images
            .iter()
            .map(|img| {
                let cpu = CpuTexture {
                    data: TextureData::RgbaU8(
                        img.rgba.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
                    ),
                    width: img.width as u32,
                    height: img.height as u32,
                    min_filter: Interpolation::Linear,
                    mag_filter: Interpolation::Linear,
                    mipmap: Some(Mipmap {
                        filter: Interpolation::Linear,
                        max_ratio: 8,
                        max_levels: 12,
                    }),
                    wrap_s: Wrapping::ClampToEdge,
                    wrap_t: Wrapping::ClampToEdge,
                    ..Default::default()
                };
                Arc::new(Texture2D::new(&self.context, &cpu))
            })
            .collect();
        self.objects.clear();
        for s in &scene.surfaces {
            if s.positions.is_empty() {
                continue;
            }
            let v3 = |v: &[f32; 3]| vec3(v[0], v[1], v[2]);
            let cpu = CpuMesh {
                positions: Positions::F32(s.positions.iter().map(v3).collect()),
                normals: Some(s.normals.iter().map(v3).collect()),
                uvs: (!s.uvs.is_empty()).then(|| s.uvs.iter().map(|u| vec2(u[0], u[1])).collect()),
                ..Default::default()
            };
            let material = Shaded {
                colour: vec3(s.colour[0], s.colour[1], s.colour[2]),
                texture: s.texture.and_then(|i| textures.get(i).cloned()),
                metal: s.metal,
                eye: vec3(0.0, 0.0, 1.0),
                key: vec3(0.0, 0.0, 1.0),
                fill: vec3(0.0, 0.0, 1.0),
            };
            self.objects.push((s.part, Gm::new(Mesh::new(&self.context, &cpu), material)));
        }
        self.scene = scene.id;
    }
}

thread_local! {
    static GPU: RefCell<Option<Gpu>> = const { RefCell::new(None) };
}

pub fn paint(ui: &Ui, rect: Rect, scene: Arc<Scene>, cam: Camera, parts: bool) {
    let callback = egui_glow::CallbackFn::new(move |info, painter| {
        GPU.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let Ok(context) = Context::from_gl_context(painter.gl().clone()) else { return };
                *slot = Some(Gpu { context, scene: 0, objects: Vec::new() });
            }
            let gpu = slot.as_mut().unwrap();
            if gpu.scene != scene.id {
                gpu.upload(&scene);
            }
            let vp = info.viewport_in_pixels();
            let clip = info.clip_rect_in_pixels();
            let size = egui::Vec2::new(vp.width_px as f32, vp.height_px as f32);
            let vw = view(&scene, &cam, size);
            let v3 = |v: [f32; 3]| vec3(v[0], v[1], v[2]);
            let viewport = Viewport {
                x: vp.left_px,
                y: vp.from_bottom_px,
                width: vp.width_px.max(1) as u32,
                height: vp.height_px.max(1) as u32,
            };
            let camera = three_d::Camera::new_perspective(
                viewport,
                v3(vw.eye),
                v3(vw.target),
                v3(vw.up),
                radians(vw.fov_y),
                vw.near,
                vw.far,
            );
            for (_, gm) in gpu.objects.iter_mut() {
                gm.material.eye = v3(vw.eye);
                gm.material.key = v3(vw.key);
                gm.material.fill = v3(vw.fill);
            }
            let x0 = clip.left_px.max(vp.left_px);
            let y0 = clip.from_bottom_px.max(vp.from_bottom_px);
            let x1 = (clip.left_px + clip.width_px).min(vp.left_px + vp.width_px);
            let y1 = (clip.from_bottom_px + clip.height_px).min(vp.from_bottom_px + vp.height_px);
            if x1 <= x0 || y1 <= y0 {
                return;
            }
            let scissor =
                ScissorBox { x: x0, y: y0, width: (x1 - x0) as u32, height: (y1 - y0) as u32 };
            let [w, h] = info.screen_size_px;
            let target = match painter.intermediate_fbo() {
                Some(fbo) => RenderTarget::from_framebuffer(&gpu.context, w, h, fbo),
                None => RenderTarget::screen(&gpu.context, w, h),
            };
            let bg = BACKGROUND.to_array().map(|c| c as f32 / 255.0);
            target.clear_partially(
                scissor,
                ClearState::color_and_depth(bg[0], bg[1], bg[2], 1.0, 1.0),
            );
            let objects = gpu.objects.iter().filter(|(p, _)| parts || !*p).map(|(_, gm)| gm);
            target.render_partially(scissor, &camera, objects, &[]);
            let _ = target.into_framebuffer();
        });
    });
    ui.painter_at(rect).add(egui::PaintCallback { rect, callback: Arc::new(callback) });
}
