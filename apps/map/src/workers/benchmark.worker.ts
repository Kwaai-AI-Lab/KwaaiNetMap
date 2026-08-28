/// <reference types="@webgpu/types" />
// Web Worker: runs WebGPU (or WebGL fallback) GEMM micro-benchmark.
// Posts progress and final results back to the main thread.

export interface BenchmarkResult {
  tokens_per_sec: number
  storage_gb: number
  cpu_cores: number
  method: 'webgpu' | 'webgl' | 'cpu'
}

export type WorkerMessage =
  | { type: 'progress'; pct: number }
  | { type: 'result'; data: BenchmarkResult }
  | { type: 'error'; message: string }

async function runWebGpuBenchmark(): Promise<number> {
  if (!navigator.gpu) throw new Error('WebGPU not available')

  const adapter = await navigator.gpu.requestAdapter()
  if (!adapter) throw new Error('No GPU adapter')
  const device = await adapter.requestDevice()

  const N = 128
  const SIZE = N * N
  const data = new Float32Array(SIZE).fill(1.0)

  const bufA = device.createBuffer({ size: SIZE * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST })
  const bufB = device.createBuffer({ size: SIZE * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST })
  const bufC = device.createBuffer({ size: SIZE * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC })

  device.queue.writeBuffer(bufA, 0, data)
  device.queue.writeBuffer(bufB, 0, data)

  const shader = /* wgsl */`
    @group(0) @binding(0) var<storage, read> A: array<f32>;
    @group(0) @binding(1) var<storage, read> B: array<f32>;
    @group(0) @binding(2) var<storage, read_write> C: array<f32>;
    @compute @workgroup_size(16, 16)
    fn main(@builtin(global_invocation_id) id: vec3<u32>) {
      let row = id.x; let col = id.y;
      let N = 128u;
      if (row >= N || col >= N) { return; }
      var sum = 0.0;
      for (var k = 0u; k < N; k++) {
        sum += A[row * N + k] * B[k * N + col];
      }
      C[row * N + col] = sum;
    }
  `

  const module = device.createShaderModule({ code: shader })
  const pipeline = await device.createComputePipelineAsync({
    layout: 'auto',
    compute: { module, entryPoint: 'main' },
  })

  const bindGroup = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: { buffer: bufA } },
      { binding: 1, resource: { buffer: bufB } },
      { binding: 2, resource: { buffer: bufC } },
    ],
  })

  const ITERS = 100
  const t0 = performance.now()
  for (let i = 0; i < ITERS; i++) {
    const enc = device.createCommandEncoder()
    const pass = enc.beginComputePass()
    pass.setPipeline(pipeline)
    pass.setBindGroup(0, bindGroup)
    pass.dispatchWorkgroups(Math.ceil(N / 16), Math.ceil(N / 16))
    pass.end()
    device.queue.submit([enc.finish()])
    if (i % 10 === 9) {
      await device.queue.onSubmittedWorkDone()
      self.postMessage({ type: 'progress', pct: Math.round((i + 1) / ITERS * 60) } satisfies WorkerMessage)
    }
  }
  await device.queue.onSubmittedWorkDone()
  const elapsed = performance.now() - t0

  // N=128 GEMM ≈ 2*128^3 = 4M FLOPs per iteration
  const flops = 2 * N ** 3 * ITERS
  const gflops = flops / (elapsed / 1000) / 1e9
  // Empirical: ~1 GFLOP/s → ~1 token/s for 7B model
  return gflops * 1.2
}

async function cpuFallbackBenchmark(): Promise<number> {
  const N = 64
  const A = new Float32Array(N * N).fill(1.0)
  const B = new Float32Array(N * N).fill(1.0)
  const C = new Float32Array(N * N)
  const ITERS = 20
  const t0 = performance.now()
  for (let iter = 0; iter < ITERS; iter++) {
    for (let i = 0; i < N; i++) {
      for (let k = 0; k < N; k++) {
        for (let j = 0; j < N; j++) {
          C[i * N + j] += A[i * N + k] * B[k * N + j]
        }
      }
    }
    self.postMessage({ type: 'progress', pct: Math.round((iter + 1) / ITERS * 60) } satisfies WorkerMessage)
  }
  const elapsed = performance.now() - t0
  const flops = 2 * N ** 3 * ITERS
  const gflops = flops / (elapsed / 1000) / 1e9
  return gflops * 0.3
}

async function main() {
  self.postMessage({ type: 'progress', pct: 5 } satisfies WorkerMessage)

  let tokens_per_sec = 0
  let method: BenchmarkResult['method'] = 'cpu'

  try {
    tokens_per_sec = await runWebGpuBenchmark()
    method = 'webgpu'
  } catch {
    try {
      tokens_per_sec = await cpuFallbackBenchmark()
      method = 'cpu'
    } catch (e) {
      self.postMessage({ type: 'error', message: String(e) } satisfies WorkerMessage)
      return
    }
  }

  self.postMessage({ type: 'progress', pct: 70 } satisfies WorkerMessage)

  // Storage estimate
  let storage_gb = 0
  try {
    const est = await navigator.storage.estimate()
    storage_gb = Math.round((est.quota ?? 0) / 1e9)
  } catch { /* ignore */ }

  self.postMessage({ type: 'progress', pct: 90 } satisfies WorkerMessage)

  const cpu_cores = navigator.hardwareConcurrency ?? 1

  self.postMessage({ type: 'progress', pct: 100 } satisfies WorkerMessage)
  self.postMessage({
    type: 'result',
    data: { tokens_per_sec: Math.round(tokens_per_sec * 10) / 10, storage_gb, cpu_cores, method },
  } satisfies WorkerMessage)
}

main()
