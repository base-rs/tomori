// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A device for tests that render offscreen, and WGSL validation for every shader, shared by this crate's tests, Sky's and the clients' through the `test-gpu` feature.
//!
//! A real GPU is preferred, and a software adapter such as Mesa's lavapipe is used when there is none, as on CI runners. Where there is no adapter at all, the test is skipped with a message on standard error, unless `TOMORI_REQUIRE_GPU` is set to anything but `0` or the empty string, as CI sets it, so that a missing driver fails the tests rather than silently skipping them.
//!
//! Every test in a test binary shares one instance, adapter and device, which are created by the first test to ask and are never dropped. Tests run on parallel threads, and before version 1.4.346 the Vulkan loader looked up a device, when one thread named an object, without the lock that another thread held while creating or destroying a device, and so read freed memory. wgpu names objects through `VK_EXT_debug_utils` in debug builds, and Ubuntu 26.04's loader, 1.4.341, crashed CI's rendering tests with SIGSEGV this way about half of the time. See <https://github.com/KhronosGroup/Vulkan-Loader/issues/1863>, fixed by <https://github.com/KhronosGroup/Vulkan-Loader/pull/1866>. With one device, nothing creates or destroys a device while tests run, and nothing is torn down at process exit. Error scopes are per thread, so tests on the same device do not see each other's errors.
use std::io::Write;
use std::sync::OnceLock;

/// The environment variable that makes a missing adapter fail the test instead of skipping it.
pub const REQUIRE_GPU: &str = "TOMORI_REQUIRE_GPU";

/// The adapter a test renders with, and a device and queue created on it with default limits, and the features of the frame profiler it has, shared by every test in the binary.
#[derive(Clone)]
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Fails if `required`, as [`REQUIRE_GPU`] says, because no adapter was found, with the `error` wgpu gave; or else returns the message that says the calling test is skipped.
fn missing(error: &dyn std::fmt::Display, required: Option<&str>) -> String {
    assert!(
        matches!(required, None | Some("" | "0")),
        "{REQUIRE_GPU} is set but no GPU adapter was found ({error}); install a Vulkan driver, such as Mesa's lavapipe (mesa-vulkan-drivers)"
    );
    format!(
        "Skipping a GPU test: no GPU adapter was found ({error}). Install a Vulkan driver, such as Mesa's lavapipe, to run it, and set {REQUIRE_GPU}=1 to fail instead."
    )
}

/// Writes to standard error directly, bypassing the test harness's capture, so that the message shows for a passing test too.
fn report(message: impl std::fmt::Display) {
    let _ = writeln!(std::io::stderr(), "{message}");
}

/// The backends tests render with: Vulkan, and Metal only when built with `--features wgpu/metal`, to preview on a Mac.
const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN.union(wgpu::Backends::METAL);

/// Opens the preferred adapter of `backends` and a device on it, or returns why there is no adapter.
///
/// # Panics
///
/// If the adapter refuses a device.
fn create(backends: wgpu::Backends) -> Result<Gpu, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // High performance orders discrete and integrated GPUs first and software (`DeviceType::Cpu`) adapters last, so a software adapter is only chosen when it is the only one.
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|error| error.to_string())?;
    let info = adapter.get_info();
    report(format_args!(
        "GPU tests use {} ({:?}, {}).",
        info.name, info.device_type, info.driver
    ));
    // With the GPU's timestamps where it has them, as lavapipe does, so that the frame profiler is tested timing.
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & crate::profiler::FEATURES,
        ..Default::default()
    }))
    .expect("the adapter must provide a device");
    Ok(Gpu {
        adapter,
        device,
        queue,
    })
}

/// The binary's shared adapter, device and queue, created on first use, or `None` after saying why the calling test is skipped.
///
/// # Panics
///
/// If no adapter is found while [`REQUIRE_GPU`] is set, or if the adapter refuses a device.
pub fn open() -> Option<Gpu> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    found(
        GPU.get_or_init(|| create(BACKENDS)),
        std::env::var(REQUIRE_GPU).ok().as_deref(),
    )
}

/// The shared GPU if there is one, or `None` after saying why the calling test is skipped; fails if `required`, as [`REQUIRE_GPU`] says.
fn found(gpu: &Result<Gpu, String>, required: Option<&str>) -> Option<Gpu> {
    match gpu {
        Ok(gpu) => Some(gpu.clone()),
        Err(error) => {
            report(missing(error, required));
            None
        }
    }
}

/// Runs `test` with the binary's shared GPU, as [`open`] gives it, and returns what it returns; or skips it, returning `None`, where there is no adapter.
///
/// # Panics
///
/// As [`open`] does.
pub fn with<T>(test: impl FnOnce(Gpu) -> T) -> Option<T> {
    open().map(test)
}

/// The `.wgsl` files directly in `directory`, by file name and in name order, with their source.
pub fn wgsl_files(directory: impl AsRef<std::path::Path>) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .expect("the shader directory must be readable")
        .map(|entry| entry.expect("the shader directory must be readable").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "wgsl")
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&path).expect("a shader must be readable");
            (name, source)
        })
        .collect();
    files.sort();
    files
}

/// Parses and validates WGSL `source` with naga, as wgpu does when it creates a shader module, so that a shader is checked where no adapter can compile it: after the shared haze functions, `tomori_assets::haze::shader`, as every pipeline that draws in the world compiles its own, unless it begins with them already or is them.
///
/// # Panics
///
/// With the error, located in the source and labelled with `label`, if the source does not parse or validate.
pub fn validate_wgsl(label: &str, source: &str) {
    let hazed;
    let source = if source.starts_with(tomori_assets::haze::WGSL) {
        source
    } else {
        hazed = tomori_assets::haze::shader(source);
        &hazed
    };
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string_with_path(source, label)));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{}", error.emit_to_string_with_path(source, label)));
    if let Some(passed) = passes_a_uniform_by_value(&module, &info) {
        panic!(
            "{label}: {passed} by value, which Tomori's shaders never do, in any stage: give a function the fields it \
             reads, or an index to read them by, as `design/rendering.md` says (the Quest 3's driver failed a multiview \
             pipeline over it, #414); a `var` counts if it is given one anywhere in the function, before or after the call, whatever else it is \
             given"
        );
    }
}

/// Where a function or an entry point of `module`, of any stage, passes by value a struct or an array loaded from a
/// uniform, as a call's argument or as its own result, said as "`function` gives `callee` …" or "`function` returns …";
/// or none. Such a value is a load of the uniform itself, an element of a uniform array or a member of a uniform
/// struct, however deeply nested, such as an eye's camera, or moved whole from one: bound by `let` or kept in a `var`,
/// whole or in a member or element of it, a member or element of such a value, or put whole in a struct or array made
/// from it. A `var` holds one if it is given one anywhere in its function, before or after the call, and whatever else
/// it is given since: following the order of stores through branches and loops, where a later store can reach an
/// earlier call, would take a flow analysis the check does not need, since a shader can name another `var`. A value
/// loaded whole and only read field by field is not passed, nor are a uniform's scalars, vectors and matrices. Storage
/// buffers and push constants are not checked.
fn passes_a_uniform_by_value(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
) -> Option<String> {
    let functions = module
        .functions
        .iter()
        .map(|(handle, function)| (function, &info[handle]));
    let entry_points = module
        .entry_points
        .iter()
        .enumerate()
        .map(|(index, entry)| (&entry.function, info.get_entry_point(index)));
    functions.chain(entry_points).find_map(|(function, info)| {
        let name = function.name.as_deref().unwrap_or_default();
        let from_uniform = |value| from_uniform(module, function, info, value, &mut Vec::new());
        find_statement(&function.body, &mut |statement| match *statement {
            naga::Statement::Call {
                function: callee,
                ref arguments,
                ..
            } => arguments.iter().find_map(|&argument| {
                from_uniform(argument).map(|uniform| {
                    let callee = module.functions[callee].name.as_deref().unwrap_or_default();
                    format!(
                        "{name} gives `{callee}` a struct or an array of the uniform `{uniform}`"
                    )
                })
            }),
            naga::Statement::Return { value: Some(value) } => from_uniform(value).map(|uniform| {
                format!("{name} returns a struct or an array of the uniform `{uniform}`")
            }),
            _ => None,
        })
    })
}

/// The first of `found` for a statement of `block`, however deeply nested in blocks, branches and loops.
fn find_statement<T>(
    block: &naga::Block,
    found: &mut impl FnMut(&naga::Statement) -> Option<T>,
) -> Option<T> {
    block.iter().find_map(|statement| {
        found(statement).or_else(|| match statement {
            naga::Statement::Block(inner) => find_statement(inner, found),
            naga::Statement::If { accept, reject, .. } => {
                find_statement(accept, found).or_else(|| find_statement(reject, found))
            }
            naga::Statement::Switch { cases, .. } => cases
                .iter()
                .find_map(|case| find_statement(&case.body, found)),
            naga::Statement::Loop {
                body, continuing, ..
            } => find_statement(body, found).or_else(|| find_statement(continuing, found)),
            _ => None,
        })
    })
}

/// The name of the uniform the struct or array `value` of `function` is loaded from or moved whole from, as
/// [`passes_a_uniform_by_value`] says, or none if it is not one, or is a scalar, vector or matrix; not following the
/// variables in `followed` again.
fn from_uniform(
    module: &naga::Module,
    function: &naga::Function,
    info: &naga::valid::FunctionInfo,
    value: naga::Handle<naga::Expression>,
    followed: &mut Vec<naga::Handle<naga::LocalVariable>>,
) -> Option<String> {
    if !matches!(
        info[value].ty.inner_with(&module.types),
        naga::TypeInner::Struct { .. } | naga::TypeInner::Array { .. }
    ) {
        return None;
    }
    match function.expressions[value] {
        naga::Expression::Load { pointer } => loaded(module, function, info, pointer, followed),
        naga::Expression::Access { base, .. } | naga::Expression::AccessIndex { base, .. } => {
            from_uniform(module, function, info, base, followed)
        }
        naga::Expression::Compose { ref components, .. } => components
            .iter()
            .find_map(|&component| from_uniform(module, function, info, component, followed)),
        _ => None,
    }
}

/// The name of the uniform `pointer` of `function` points into, through elements and members however deeply, or into
/// a variable of `function` given a struct or array from one anywhere in it, whole or in a member or element, as
/// [`from_uniform`] says; or none if it points elsewhere.
fn loaded(
    module: &naga::Module,
    function: &naga::Function,
    info: &naga::valid::FunctionInfo,
    pointer: naga::Handle<naga::Expression>,
    followed: &mut Vec<naga::Handle<naga::LocalVariable>>,
) -> Option<String> {
    match function.expressions[pointer] {
        naga::Expression::GlobalVariable(global) => {
            let global = &module.global_variables[global];
            (global.space == naga::AddressSpace::Uniform)
                .then(|| global.name.clone().unwrap_or_default())
        }
        naga::Expression::Access { base, .. } | naga::Expression::AccessIndex { base, .. } => {
            loaded(module, function, info, base, followed)
        }
        naga::Expression::LocalVariable(local) if !followed.contains(&local) => {
            followed.push(local);
            let mut values = Vec::from_iter(function.local_variables[local].init);
            find_statement(&function.body, &mut |statement| {
                if let naga::Statement::Store { pointer, value } = *statement
                    && variable(function, pointer) == Some(local)
                {
                    values.push(value);
                }
                None::<()>
            });
            values
                .into_iter()
                .find_map(|value| from_uniform(module, function, info, value, followed))
        }
        _ => None,
    }
}

/// The variable of `function` that `pointer` points into, whole or through its members and elements however deeply, or
/// none if it points elsewhere.
fn variable(
    function: &naga::Function,
    pointer: naga::Handle<naga::Expression>,
) -> Option<naga::Handle<naga::LocalVariable>> {
    match function.expressions[pointer] {
        naga::Expression::LocalVariable(local) => Some(local),
        naga::Expression::Access { base, .. } | naga::Expression::AccessIndex { base, .. } => {
            variable(function, base)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_adapter_is_skipped_unless_required() {
        for required in [None, Some(""), Some("0")] {
            assert!(
                missing(&"no adapter", required)
                    .starts_with("Skipping a GPU test: no GPU adapter was found (no adapter).")
            );
        }
    }

    #[test]
    #[should_panic(
        expected = "TOMORI_REQUIRE_GPU is set but no GPU adapter was found (no adapter)"
    )]
    fn a_missing_adapter_fails_when_required() {
        missing(&"no adapter", Some("1"));
    }

    #[test]
    fn a_test_is_skipped_without_an_adapter() {
        let gpu = create(wgpu::Backends::empty());
        assert!(gpu.is_err());
        assert!(found(&gpu, None).is_none());
    }

    #[test]
    fn a_test_runs_with_the_shared_gpu() {
        with(|gpu| {
            let name = gpu.adapter.get_info().name;
            let again = found(&Ok(gpu), Some("1")).expect("the GPU found");
            assert_eq!(again.adapter.get_info().name, name);
        });
    }

    #[test]
    #[should_panic(
        expected = "twice gives `grown` a struct or an array of the uniform `camera` by value"
    )]
    fn a_vertex_shader_passing_its_uniform_is_refused() {
        validate_wgsl(
            "camera.wgsl",
            "struct Camera { matrix: mat4x4<f32>, pixels: f32 }
@group(0) @binding(0) var<uniform> camera: Camera;
fn grown(camera: Camera) -> f32 { return camera.pixels; }
fn twice() -> f32 { return grown(camera) * 2.0; }
@vertex
fn vs_main(@location(0) at: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.matrix * vec4<f32>(at * twice(), 1.0);
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "vs_main gives `grown` a struct or an array of the uniform `cameras` by value"
    )]
    fn a_vertex_shader_passing_an_eye_s_camera_bound_by_let_is_refused() {
        // As the Quest 3's driver failed for both eyes by multiview: the camera of the view, by a dynamic index, bound
        // by `let`, then given to a function.
        validate_wgsl(
            "cameras.wgsl",
            "struct Camera { matrix: mat4x4<f32>, pixels: f32 }
@group(0) @binding(0) var<uniform> cameras: array<Camera, 2>;
fn grown(camera: Camera) -> f32 { return camera.pixels; }
@vertex
fn vs_main(@builtin(instance_index) view: u32, @location(0) at: vec3<f32>) -> @builtin(position) vec4<f32> {
    let camera = cameras[view % 2u];
    return camera.matrix * vec4<f32>(at * grown(camera), 1.0);
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "fs_main gives `lit` a struct or an array of the uniform `globals` by value"
    )]
    fn a_fragment_shader_passing_a_struct_nested_in_a_uniform_is_refused() {
        validate_wgsl(
            "nested.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
struct Lights { key: Light, fill: Light }
struct Globals { time: f32, lights: Lights }
@group(0) @binding(0) var<uniform> globals: Globals;
fn lit(light: Light) -> vec4<f32> { return light.colour * light.strength; }
@fragment
fn fs_main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    if at.x > globals.time {
        return lit(globals.lights.fill);
    }
    return vec4<f32>(0.0);
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "cs_main gives `sum` a struct or an array of the uniform `settings` by value"
    )]
    fn a_compute_shader_passing_a_uniform_array_in_a_loop_is_refused() {
        validate_wgsl(
            "offsets.wgsl",
            "struct Settings { scale: f32, offsets: array<vec4<f32>, 4> }
@group(0) @binding(0) var<uniform> settings: Settings;
@group(0) @binding(1) var<storage, read_write> out: array<f32>;
fn sum(offsets: array<vec4<f32>, 4>) -> f32 { return offsets[0].x + offsets[3].x; }
@compute @workgroup_size(1)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    for (var index = 0u; index < id.x; index += 1u) {
        out[index] = sum(settings.offsets);
    }
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "light returns a struct or an array of the uniform `lights` by value"
    )]
    fn a_fragment_shader_returning_an_element_of_nested_uniform_arrays_is_refused() {
        // A function's result leaves it by value as an argument enters one.
        validate_wgsl(
            "arrays.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
struct Lights { all: array<array<Light, 2>, 2> }
@group(0) @binding(0) var<uniform> lights: Lights;
fn light(index: u32) -> Light { return lights.all[index % 2u][1]; }
@fragment
fn fs_main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    return light(u32(at.x)).colour;
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "cs_main gives `lit` a struct or an array of the uniform `globals` by value"
    )]
    fn a_compute_shader_passing_a_member_of_a_uniform_copied_into_a_variable_is_refused() {
        // Moved whole by `let`, then into a variable, then a member of it given to a function.
        validate_wgsl(
            "copy.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
struct Globals { time: f32, light: Light }
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read_write> out: array<vec4<f32>>;
fn lit(light: Light) -> vec4<f32> { return light.colour * light.strength; }
@compute @workgroup_size(1)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let loaded = globals;
    var copy: Globals;
    copy.time = 1.0;
    copy = loaded;
    out[id.x] = lit(copy.light);
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "fs_main gives `lit` a struct or an array of the uniform `globals` by value"
    )]
    fn a_fragment_shader_passing_a_member_of_a_variable_given_a_uniform_s_struct_is_refused() {
        // Stored into a member of a variable, not the variable whole, then given to a function in a block of its own.
        validate_wgsl(
            "member.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
struct Globals { time: f32, light: Light }
@group(0) @binding(0) var<uniform> globals: Globals;
fn lit(light: Light) -> vec4<f32> { return light.colour * light.strength; }
@fragment
fn fs_main() -> @location(0) vec4<f32> {
    var copy: Globals;
    copy.time = 1.0;
    copy.light = globals.light;
    {
        return lit(copy.light) * copy.time;
    }
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "cs_main gives `brightest` a struct or an array of the uniform `lights` by value"
    )]
    fn a_compute_shader_passing_an_array_with_an_element_from_a_uniform_is_refused() {
        // Stored into an element of an array variable, by a dynamic index, then the array given to a function.
        validate_wgsl(
            "element.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
@group(0) @binding(0) var<uniform> lights: array<Light, 4>;
@group(0) @binding(1) var<storage, read_write> out: array<f32>;
fn brightest(chosen: array<Light, 2>) -> f32 { return max(chosen[0].strength, chosen[1].strength); }
@compute @workgroup_size(1)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    var chosen: array<Light, 2>;
    chosen[id.x % 2u] = lights[id.x % 4u];
    out[id.x] = brightest(chosen);
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "vs_main gives `lit` a struct or an array of the uniform `globals` by value"
    )]
    fn a_vertex_shader_passing_a_variable_once_given_a_uniform_s_struct_is_refused() {
        // Conservatively: the variable is given a struct made in the shader before the call, but it was given the
        // uniform's, so it is refused whatever the order of its stores.
        validate_wgsl(
            "overwritten.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
struct Globals { time: f32, light: Light }
@group(0) @binding(0) var<uniform> globals: Globals;
fn lit(light: Light) -> vec4<f32> { return light.colour * light.strength; }
@vertex
fn vs_main() -> @builtin(position) vec4<f32> {
    var light = globals.light;
    light = Light(vec4<f32>(1.0), 1.0);
    return lit(light);
}
",
        );
    }

    #[test]
    #[should_panic(
        expected = "fs_main gives `lit` a struct or an array of the uniform `globals` by value"
    )]
    fn a_fragment_shader_passing_a_uniform_s_struct_put_in_another_is_refused() {
        // A member of a value loaded whole, put whole in a struct made in the shader, in a switch.
        validate_wgsl(
            "wrapped.wgsl",
            "struct Light { colour: vec4<f32>, strength: f32 }
struct Globals { time: f32, light: Light }
struct Wrapped { light: Light, weight: f32 }
@group(0) @binding(0) var<uniform> globals: Globals;
fn lit(wrapped: Wrapped) -> vec4<f32> { return wrapped.light.colour * wrapped.weight; }
@fragment
fn fs_main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let loaded = globals;
    switch u32(at.x) {
        case 0u: { return lit(Wrapped(loaded.light, 0.5)); }
        default: { return vec4<f32>(0.0); }
    }
}
",
        );
    }

    #[test]
    fn shaders_reading_a_uniform_s_fields_or_passing_their_own_values_are_valid() {
        // As in #414, an eye's camera loaded whole by `let`, by a dynamic index, and read field by field, its fields
        // given to a function, and a copy of a uniform in a variable read field by field, in every stage; a struct
        // rebuilt from a uniform's fields, a struct made in the shader, or a copy of one, an argument passed on, a
        // struct made and returned, and a struct read whole from a storage buffer are not passed from a uniform.
        validate_wgsl(
            "fields.wgsl",
            "struct Camera { matrix: mat4x4<f32>, pixels: f32, eye: vec4<f32>, offsets: array<vec4<f32>, 2> }
struct Eye { matrix: mat4x4<f32>, eye: vec4<f32> }
@group(0) @binding(0) var<uniform> time: f32;
@group(0) @binding(1) var<uniform> cameras: array<Camera, 2>;
@group(0) @binding(2) var<storage, read_write> stored: Camera;
fn twice(value: f32) -> f32 { return value * 2.0; }
fn placed(matrix: mat4x4<f32>, eye: vec3<f32>, at: vec3<f32>) -> vec4<f32> { return matrix * vec4<f32>(at - eye, 1.0); }
fn seen(eye: Eye) -> vec4<f32> { return eye.eye; }
fn grown(camera: Camera) -> f32 { return camera.pixels; }
fn passed(camera: Camera) -> f32 { return grown(camera); }
fn eye(view: u32) -> Eye { return Eye(cameras[view].matrix, cameras[view].eye); }
@vertex
fn vs_main(@builtin(instance_index) view: u32, @location(0) at: vec3<f32>) -> @builtin(position) vec4<f32> {
    let camera = cameras[view];
    var made = Camera(camera.matrix, twice(time), camera.eye, array(camera.offsets[0], camera.offsets[view % 2u]));
    made = made;
    let grown = passed(made) + grown(stored) + seen(eye(view)).w;
    return placed(camera.matrix, camera.eye.xyz, at * twice(camera.pixels) * grown);
}
@fragment
fn fs_main() -> @location(0) vec4<f32> {
    var copy = cameras[1];
    return copy.matrix[0] * copy.pixels + cameras[0].offsets[1];
}
@compute @workgroup_size(1)
fn cs_main() {
    stored.pixels = cameras[0].pixels * time;
}
",
        );
    }

    #[test]
    #[should_panic(expected = "broken.wgsl")]
    fn invalid_wgsl_names_the_shader() {
        validate_wgsl("broken.wgsl", "fn main() -> f32 { return true; }");
    }

    #[test]
    #[should_panic(expected = "unparsable.wgsl")]
    fn unparsable_wgsl_names_the_shader() {
        validate_wgsl("unparsable.wgsl", "fn main( {");
    }
}
