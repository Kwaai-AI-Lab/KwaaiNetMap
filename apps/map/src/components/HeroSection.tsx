import { useEffect, useRef, useState } from 'react'
import type { NetworkStats, NodeEntry } from '../lib/api'
import { NetworkGraph } from './NetworkGraph'

interface Props {
  stats: NetworkStats
  nodes: NodeEntry[]
  connected: boolean
  onBenchmark: () => void
}

function AnimatedCounter({ value, decimals = 0 }: { value: number; decimals?: number }) {
  const [display, setDisplay] = useState(value)
  const prev = useRef(value)

  useEffect(() => {
    const from = prev.current
    const to = value
    const duration = 600
    const start = performance.now()

    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1)
      const eased = 1 - Math.pow(1 - t, 3)
      setDisplay(from + (to - from) * eased)
      if (t < 1) requestAnimationFrame(tick)
      else prev.current = to
    }
    requestAnimationFrame(tick)
  }, [value])

  return <>{display.toFixed(decimals)}</>
}

const STAT_CARDS = [
  { label: 'Nodes', key: 'node_count' as const, suffix: '', decimals: 0 },
  { label: 'Tokens/sec', key: 'tokens_per_sec' as const, suffix: '', decimals: 1 },
  { label: 'Coverage', key: 'coverage_pct' as const, suffix: '%', decimals: 1 },
  { label: 'Active', key: 'active_sessions' as const, suffix: '', decimals: 0 },
]

export function HeroSection({ stats, nodes, connected, onBenchmark }: Props) {
  return (
    <section className="relative min-h-screen flex flex-col items-center justify-center px-4 py-20">
      {/* Background graph */}
      <div className="absolute inset-0 opacity-40 pointer-events-none">
        <NetworkGraph nodes={nodes} height={typeof window !== 'undefined' ? window.innerHeight : 600} />
      </div>

      {/* Content */}
      <div className="relative z-10 text-center max-w-4xl mx-auto">
        {/* Logo */}
        <div className="flex justify-center mb-8">
          <img src="/kwaai-logo.png" alt="Kwaai" className="h-20 w-auto" />
        </div>

        {/* Live indicator */}
        <div className="flex items-center justify-center gap-2 mb-6">
          <span
            className={`inline-block w-2 h-2 rounded-full ${connected ? 'bg-kwaai-green animate-pulse' : 'bg-kwaai-amber'}`}
          />
          <span className="text-xs font-mono text-kwaai-green/80 uppercase tracking-widest">
            {connected ? 'Live network' : 'Connecting…'}
          </span>
        </div>

        {/* Headline */}
        <h1 className="text-5xl md:text-7xl font-bold mb-4 leading-tight">
          <span className="gradient-text">Your compute.</span>
          <br />
          <span className="text-white">Your AI. Yours.</span>
        </h1>
        <p className="text-lg md:text-xl text-slate-400 mb-12 max-w-2xl mx-auto">
          Decentralized inference for the open web. Join the network, earn rewards, keep
          control.
        </p>

        {/* Live stats */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-12">
          {STAT_CARDS.map(({ label, key, suffix, decimals }) => (
            <div key={key} className="glass px-4 py-5 text-center">
              <div className="text-3xl font-bold font-mono text-white">
                <AnimatedCounter value={stats[key]} decimals={decimals} />
                {suffix}
              </div>
              <div className="text-xs text-slate-400 mt-1 uppercase tracking-wide">{label}</div>
            </div>
          ))}
        </div>

        {/* CTAs */}
        <div className="flex flex-col sm:flex-row gap-4 justify-center">
          <button
            onClick={onBenchmark}
            className="px-8 py-4 rounded-xl font-semibold text-white bg-kwaai-blue hover:bg-blue-500 transition-colors shadow-lg shadow-blue-500/20"
          >
            Benchmark my machine
          </button>
          <a
            href="#network"
            className="px-8 py-4 rounded-xl font-semibold text-slate-300 border border-slate-700 hover:border-kwaai-blue/50 hover:text-white transition-colors"
          >
            View network
          </a>
        </div>
      </div>

      {/* Scroll hint */}
      <div className="absolute bottom-8 left-1/2 -translate-x-1/2 opacity-40">
        <div className="w-5 h-8 border-2 border-slate-500 rounded-full flex items-start justify-center pt-1">
          <div className="w-1 h-2 bg-slate-400 rounded-full animate-bounce" />
        </div>
      </div>
    </section>
  )
}
