//! WebGPU rendering backend for WASM-compiled MAGI programs.
//!
//! Generates WASM import stubs that bind to the browser WebGPU API via
//! a JavaScript host. When a MAGI program calls `gpu_init()`, `gpu_draw()`, etc.,
//! the WASM module imports these from the `"webgpu"` namespace and the JS host
//! bridges them to `navigator.gpu`.
//!
//! Architecture:
//!   MAGI source → WASM binary (with WebGPU imports) → Browser JS host → WebGPU API

use super::wasm_binary::{ValType, TypeSection, ImportSection, EntityType};

/// WebGPU import function signatures.
pub struct WebGpuBindings;

/// A WebGPU import descriptor.
pub struct GpuImport {
    pub name: &'static str,
    pub params: &'static [ValType],
    pub results: &'static [ValType],
}

const GPU_IMPORTS: &[GpuImport] = &[
    GpuImport { name: "gpu_init", params: &[ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_create_buffer", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_create_shader", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_create_pipeline", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_begin_render_pass", params: &[ValType::I32, ValType::F32, ValType::F32, ValType::F32, ValType::F32], results: &[ValType::I32] },
    GpuImport { name: "gpu_set_pipeline", params: &[ValType::I32, ValType::I32], results: &[] },
    GpuImport { name: "gpu_set_vertex_buffer", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[] },
    GpuImport { name: "gpu_draw", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[] },
    GpuImport { name: "gpu_end_render_pass", params: &[ValType::I32], results: &[] },
    GpuImport { name: "gpu_submit", params: &[ValType::I32], results: &[] },
    GpuImport { name: "gpu_present", params: &[ValType::I32], results: &[] },
    GpuImport { name: "gpu_write_buffer", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[] },
    GpuImport { name: "gpu_create_texture", params: &[ValType::I32, ValType::I32, ValType::I32, ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_create_command_encoder", params: &[ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_destroy", params: &[ValType::I32], results: &[] },
    GpuImport { name: "gpu_set_bind_group", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[] },
    GpuImport { name: "gpu_create_bind_group", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[ValType::I32] },
    GpuImport { name: "gpu_draw_indexed", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[] },
    GpuImport { name: "gpu_set_index_buffer", params: &[ValType::I32, ValType::I32, ValType::I32], results: &[] },
];

impl WebGpuBindings {
    /// Returns all GPU import descriptors.
    pub fn imports() -> &'static [GpuImport] {
        GPU_IMPORTS
    }

    /// Inject WebGPU imports into type and import sections.
    /// `base_type_idx` is the next available type index.
    pub fn inject(types: &mut TypeSection, imports: &mut ImportSection, base_type_idx: u32) {
        for (i, import) in GPU_IMPORTS.iter().enumerate() {
            types.ty().function(import.params.to_vec(), import.results.to_vec());
            imports.import("webgpu", import.name, EntityType::Function(base_type_idx + i as u32));
        }
    }

    /// Generate the JavaScript host bindings that bridge WASM imports to WebGPU.
    pub fn js_host_bindings() -> &'static str {
        r#"// MAGI WebGPU host bindings — auto-generated
// Load this alongside the WASM module to provide WebGPU access.

class MagiWebGPU {
  constructor() {
    this.handles = new Map();
    this.nextHandle = 1;
    this.device = null;
    this.context = null;
    this.encoder = null;
  }

  allocHandle(obj) {
    const h = this.nextHandle++;
    this.handles.set(h, obj);
    return h;
  }

  getHandle(h) { return this.handles.get(h); }

  async init(canvasSelector) {
    if (!navigator.gpu) throw new Error("WebGPU not supported");
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) throw new Error("No GPU adapter found");
    this.device = await adapter.requestDevice();
    const canvas = document.querySelector(canvasSelector || "canvas");
    if (canvas) {
      this.context = canvas.getContext("webgpu");
      this.context.configure({
        device: this.device,
        format: navigator.gpu.getPreferredCanvasFormat(),
      });
    }
    return this.allocHandle(this.device);
  }

  getImports(memory) {
    const gpu = this;
    return {
      webgpu: {
        gpu_init(canvasId) { return gpu.allocHandle(gpu.device); },
        gpu_create_buffer(deviceH, size, usage) {
          const buf = gpu.getHandle(deviceH).createBuffer({ size, usage, mappedAtCreation: false });
          return gpu.allocHandle(buf);
        },
        gpu_create_shader(deviceH, codePtr, codeLen) {
          const code = new TextDecoder().decode(new Uint8Array(memory.buffer, codePtr, codeLen));
          return gpu.allocHandle(gpu.getHandle(deviceH).createShaderModule({ code }));
        },
        gpu_create_pipeline(deviceH, vertexH, fragmentH) {
          const device = gpu.getHandle(deviceH);
          const format = navigator.gpu.getPreferredCanvasFormat();
          return gpu.allocHandle(device.createRenderPipeline({
            layout: "auto",
            vertex: { module: gpu.getHandle(vertexH), entryPoint: "vs_main" },
            fragment: { module: gpu.getHandle(fragmentH), entryPoint: "fs_main", targets: [{ format }] },
          }));
        },
        gpu_begin_render_pass(encoderH, r, g, b, a) {
          const view = gpu.context.getCurrentTexture().createView();
          return gpu.allocHandle(gpu.getHandle(encoderH).beginRenderPass({
            colorAttachments: [{ view, clearValue: { r, g, b, a }, loadOp: "clear", storeOp: "store" }],
          }));
        },
        gpu_set_pipeline(passH, pipelineH) { gpu.getHandle(passH).setPipeline(gpu.getHandle(pipelineH)); },
        gpu_set_vertex_buffer(passH, slot, bufferH) { gpu.getHandle(passH).setVertexBuffer(slot, gpu.getHandle(bufferH)); },
        gpu_draw(passH, vertexCount, instanceCount) { gpu.getHandle(passH).draw(vertexCount, instanceCount); },
        gpu_end_render_pass(passH) { gpu.getHandle(passH).end(); },
        gpu_submit(deviceH) {
          if (gpu.encoder) {
            gpu.getHandle(deviceH).queue.submit([gpu.encoder.finish()]);
            gpu.encoder = null;
          }
        },
        gpu_present() { /* browser auto-presents */ },
        gpu_write_buffer(bufferH, dataPtr, dataLen) {
          gpu.device.queue.writeBuffer(gpu.getHandle(bufferH), 0, new Uint8Array(memory.buffer, dataPtr, dataLen));
        },
        gpu_create_texture(deviceH, width, height, _format) {
          return gpu.allocHandle(gpu.getHandle(deviceH).createTexture({
            size: [width, height, 1], format: "rgba8unorm",
            usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT,
          }));
        },
        gpu_create_command_encoder(deviceH) {
          gpu.encoder = gpu.getHandle(deviceH).createCommandEncoder();
          return gpu.allocHandle(gpu.encoder);
        },
        gpu_destroy(deviceH) { const d = gpu.getHandle(deviceH); if (d?.destroy) d.destroy(); gpu.handles.delete(deviceH); },
        gpu_set_bind_group(passH, index, groupH) { gpu.getHandle(passH).setBindGroup(index, gpu.getHandle(groupH)); },
        gpu_create_bind_group(deviceH, layoutH, bufferH) {
          return gpu.allocHandle(gpu.getHandle(deviceH).createBindGroup({
            layout: gpu.getHandle(layoutH), entries: [{ binding: 0, resource: { buffer: gpu.getHandle(bufferH) } }],
          }));
        },
        gpu_draw_indexed(passH, indexCount, instanceCount) { gpu.getHandle(passH).drawIndexed(indexCount, instanceCount); },
        gpu_set_index_buffer(passH, bufferH, format) {
          gpu.getHandle(passH).setIndexBuffer(gpu.getHandle(bufferH), format === 0 ? "uint16" : "uint32");
        },
      },
    };
  }
}
"#
    }
}

/// Buffer usage flags (mirror WebGPU GPUBufferUsage).
pub mod buffer_usage {
    pub const MAP_READ: u32 = 0x0001;
    pub const MAP_WRITE: u32 = 0x0002;
    pub const COPY_SRC: u32 = 0x0004;
    pub const COPY_DST: u32 = 0x0008;
    pub const INDEX: u32 = 0x0010;
    pub const VERTEX: u32 = 0x0020;
    pub const UNIFORM: u32 = 0x0040;
    pub const STORAGE: u32 = 0x0080;
}

/// Texture format constants.
pub mod texture_format {
    pub const RGBA8_UNORM: u32 = 0;
    pub const BGRA8_UNORM: u32 = 1;
    pub const RGBA16_FLOAT: u32 = 2;
    pub const DEPTH24_PLUS: u32 = 3;
}
