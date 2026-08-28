// Typed API wrappers for map-server endpoints

const BASE = import.meta.env.VITE_API_BASE ?? ''

export interface NetworkStats {
  node_count: number
  tokens_per_sec: number
  coverage_pct: number
  active_sessions: number
}

export interface NodeEntry {
  peer_id: string
  trust_tier: 'Unknown' | 'Known' | 'Verified' | 'Trusted'
  start_block: number
  end_block: number
  throughput: number
  public_name: string
  version: string
  vpk: boolean
  last_seen: string
}

export async function fetchStats(): Promise<NetworkStats> {
  const res = await fetch(`${BASE}/api/stats`)
  if (!res.ok) throw new Error(`stats fetch failed: ${res.status}`)
  return res.json()
}

export async function fetchNodes(): Promise<NodeEntry[]> {
  const res = await fetch(`${BASE}/api/nodes`)
  if (!res.ok) throw new Error(`nodes fetch failed: ${res.status}`)
  return res.json()
}

export function wsLiveUrl(): string {
  const base = import.meta.env.VITE_API_BASE ?? ''
  if (base.startsWith('https://')) {
    return base.replace('https://', 'wss://') + '/api/live'
  }
  if (base.startsWith('http://')) {
    return base.replace('http://', 'ws://') + '/api/live'
  }
  // Relative — use current host
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${window.location.host}/api/live`
}
