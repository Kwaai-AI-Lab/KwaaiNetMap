import { useRef } from 'react'
import { useNetworkStats } from './hooks/useNetworkStats'
import { HeroSection } from './components/HeroSection'
import { BenchmarkSection } from './components/BenchmarkSection'
import { TrustGraphSection } from './components/TrustGraphSection'
import { InstallSection } from './components/InstallSection'

export default function App() {
  const { stats, nodes, connected } = useNetworkStats()
  const benchmarkRef = useRef<HTMLElement>(null)

  const scrollToBenchmark = () => {
    benchmarkRef.current?.scrollIntoView({ behavior: 'smooth' })
  }

  return (
    <div className="min-h-screen bg-bg-deep text-white">
      {/* Nav */}
      <nav className="fixed top-0 left-0 right-0 z-50 flex items-center justify-between px-6 py-4 bg-bg-deep/80 backdrop-blur border-b border-slate-800/50">
        <div className="flex items-center gap-3">
          <img src="/kwaai-logo.png" alt="Kwaai" className="h-9 w-auto" />
          <span className="text-slate-600 text-xs hidden sm:block">/ map.kwaai.ai</span>
        </div>
        <div className="flex items-center gap-6 text-sm">
          <a href="#network" className="text-slate-400 hover:text-white transition-colors hidden sm:block">
            Network
          </a>
          <a href="#benchmark" className="text-slate-400 hover:text-white transition-colors hidden sm:block">
            Benchmark
          </a>
          <a
            href="#install"
            className="px-4 py-2 rounded-lg bg-kwaai-blue/10 border border-kwaai-blue/30 text-kwaai-blue hover:bg-kwaai-blue/20 transition-colors"
          >
            Join
          </a>
        </div>
      </nav>

      {/* Main content */}
      <main className="pt-16">
        <HeroSection
          stats={stats}
          nodes={nodes}
          connected={connected}
          onBenchmark={scrollToBenchmark}
        />

        <BenchmarkSection benchmarkRef={benchmarkRef as React.RefObject<HTMLElement>} />

        <TrustGraphSection nodes={nodes} />

        <InstallSection />
      </main>

      {/* Footer */}
      <footer className="border-t border-slate-800/50 py-8 px-4 text-center text-slate-500 text-sm">
        <div className="flex justify-center mb-4">
          <img src="/kwaai-logo.png" alt="Kwaai" className="h-10 w-auto opacity-60" />
        </div>
        <p>
          KwaaiNet by{' '}
          <a
            href="https://github.com/Kwaai-AI-Lab"
            target="_blank"
            rel="noreferrer"
            className="text-kwaai-blue hover:text-blue-400 transition-colors"
          >
            Kwaai AI Lab
          </a>
          {' '} · MIT License · Trust framework aligned with{' '}
          <a
            href="https://trustoverip.org"
            target="_blank"
            rel="noreferrer"
            className="text-kwaai-blue hover:text-blue-400 transition-colors"
          >
            Linux Foundation ToIP
          </a>
        </p>
      </footer>
    </div>
  )
}
