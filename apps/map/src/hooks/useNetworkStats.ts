import { useEffect, useState } from 'react'
import useWebSocket, { ReadyState } from 'react-use-websocket'
import { fetchStats, fetchNodes, wsLiveUrl } from '../lib/api'
import type { NetworkStats, NodeEntry } from '../lib/api'

const FALLBACK_STATS: NetworkStats = {
  node_count: 0,
  tokens_per_sec: 0,
  coverage_pct: 0,
  active_sessions: 0,
}

export function useNetworkStats() {
  const [stats, setStats] = useState<NetworkStats>(FALLBACK_STATS)
  const [nodes, setNodes] = useState<NodeEntry[]>([])
  const [connected, setConnected] = useState(false)

  // Initial HTTP fetch for nodes list (refreshed every 30 s)
  useEffect(() => {
    const load = async () => {
      try {
        const [s, n] = await Promise.all([fetchStats(), fetchNodes()])
        setStats(s)
        setNodes(n)
      } catch {
        // Backend not yet available (dev mode without map-server)
      }
    }
    load()
    const id = setInterval(load, 30_000)
    return () => clearInterval(id)
  }, [])

  // WebSocket for live stat deltas
  const { lastMessage, readyState } = useWebSocket(wsLiveUrl(), {
    shouldReconnect: () => true,
    reconnectAttempts: 999,
    reconnectInterval: 3000,
  })

  useEffect(() => {
    setConnected(readyState === ReadyState.OPEN)
  }, [readyState])

  useEffect(() => {
    if (!lastMessage?.data) return
    try {
      const update = JSON.parse(lastMessage.data as string) as NetworkStats
      setStats(update)
    } catch {
      // ignore malformed frames
    }
  }, [lastMessage])

  return { stats, nodes, connected }
}
