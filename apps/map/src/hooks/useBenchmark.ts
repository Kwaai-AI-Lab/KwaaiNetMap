import { useCallback, useRef, useState } from 'react'
import type { BenchmarkResult, WorkerMessage } from '../workers/benchmark.worker'

export interface BenchmarkState {
  running: boolean
  progress: number
  results: BenchmarkResult | null
  error: string | null
}

export interface ExternalDrive {
  name: string
  pledgedGb: number
}

// ~$1.50 per sustained token/sec per month based on current network demand
const MONTHLY_RATE_PER_TPS = 1.5
// ~$0.005 per GB pledged per month (rough VPK storage rate)
const MONTHLY_RATE_PER_GB = 0.005

export function estimateEarnings(tps: number, storageGb = 0, externalGb = 0): { low: number; high: number } {
  const computeMid = tps * MONTHLY_RATE_PER_TPS * 0.8
  const storageMid = (storageGb + externalGb) * MONTHLY_RATE_PER_GB * 0.8
  const mid = computeMid + storageMid
  return { low: Math.round(mid * 0.7), high: Math.round(mid * 1.3) }
}

export function useExternalStorage() {
  const [drives, setDrives] = useState<ExternalDrive[]>([])
  const [requesting, setRequesting] = useState(false)

  const addDrive = useCallback(async () => {
    if (!('showDirectoryPicker' in window)) return
    setRequesting(true)
    try {
      // id: Chrome remembers this location across opens (starts in last-used dir)
      // mode: 'read' — we only verify access, never write
      // No 'startIn: external' exists in the spec; macOS NSOpenPanel shows
      // external drives in the sidebar under Locations automatically.
      const handle = await (window as Window & {
        showDirectoryPicker(opts?: { id?: string; mode?: string }): Promise<FileSystemDirectoryHandle>
      }).showDirectoryPicker({ id: 'kwaai-storage', mode: 'read' })
      setDrives(prev => {
        if (prev.some(d => d.name === handle.name)) return prev
        return [...prev, { name: handle.name, pledgedGb: 500 }]
      })
    } catch {
      // user cancelled or permission denied — silent
    } finally {
      setRequesting(false)
    }
  }, [])

  const setPledge = useCallback((name: string, gb: number) => {
    setDrives(prev => prev.map(d => d.name === name ? { ...d, pledgedGb: gb } : d))
  }, [])

  const removeDrive = useCallback((name: string) => {
    setDrives(prev => prev.filter(d => d.name !== name))
  }, [])

  const totalExternalGb = drives.reduce((s, d) => s + d.pledgedGb, 0)

  return { drives, requesting, addDrive, setPledge, removeDrive, totalExternalGb }
}

export function useBenchmark() {
  const [state, setState] = useState<BenchmarkState>({
    running: false,
    progress: 0,
    results: null,
    error: null,
  })
  const workerRef = useRef<Worker | null>(null)

  const start = useCallback(() => {
    if (state.running) return
    setState({ running: true, progress: 0, results: null, error: null })

    const worker = new Worker(
      new URL('../workers/benchmark.worker.ts', import.meta.url),
      { type: 'module' }
    )
    workerRef.current = worker

    worker.onmessage = (e: MessageEvent<WorkerMessage>) => {
      const msg = e.data
      if (msg.type === 'progress') {
        setState(s => ({ ...s, progress: msg.pct }))
      } else if (msg.type === 'result') {
        setState({ running: false, progress: 100, results: msg.data, error: null })
        worker.terminate()
      } else if (msg.type === 'error') {
        setState({ running: false, progress: 0, results: null, error: msg.message })
        worker.terminate()
      }
    }

    worker.onerror = (e) => {
      setState({ running: false, progress: 0, results: null, error: e.message })
      worker.terminate()
    }
  }, [state.running])

  return { ...state, start }
}
