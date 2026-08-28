import { useEffect, useRef } from 'react'
import toast from 'react-hot-toast'
import { useBenchmark, useExternalStorage, estimateEarnings } from '../hooks/useBenchmark'

interface Props {
  benchmarkRef?: React.RefObject<HTMLElement>
}

function ProgressBar({ value, color }: { value: number; color: string }) {
  return (
    <div className="w-full bg-slate-800 rounded-full h-2 overflow-hidden">
      <div
        className="h-full rounded-full transition-all duration-300"
        style={{ width: `${Math.min(100, value)}%`, background: color }}
      />
    </div>
  )
}

const fspSupported = typeof window !== 'undefined' && 'showDirectoryPicker' in window

export function BenchmarkSection({ benchmarkRef }: Props) {
  const { running, progress, results, error, start: runBenchmark } = useBenchmark()
  const { drives, requesting, addDrive, setPledge, removeDrive, totalExternalGb } = useExternalStorage()
  const toastedRef = useRef(false)

  const earnings = results
    ? estimateEarnings(results.tokens_per_sec, results.storage_gb, totalExternalGb)
    : null

  useEffect(() => {
    if (results && !running && !toastedRef.current && earnings) {
      toastedRef.current = true
      if (earnings.low > 0) {
        toast.success(`Machine scanned ✓ — potential: $${earnings.low}–$${earnings.high}/mo`, { duration: 5000 })
      }
    }
    if (!results) toastedRef.current = false
  }, [results, running, earnings])

  // Re-fire toast when external storage changes after benchmark
  useEffect(() => {
    if (results && !running && earnings && drives.length > 0) {
      toast.success(`Storage updated — new potential: $${earnings.low}–$${earnings.high}/mo`, { duration: 3000 })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [totalExternalGb])

  return (
    <section id="benchmark" ref={benchmarkRef} className="py-24 px-4">
      <div className="max-w-2xl mx-auto">
        <h2 className="text-3xl md:text-4xl font-bold text-center mb-2">
          What could you earn?
        </h2>
        <p className="text-slate-400 text-center mb-12">
          We benchmark your machine and estimate monthly earnings based on live network demand.
        </p>

        <div className="glass p-8 space-y-6">
          {/* ── Pre-run state ── */}
          {!results && !running && !error && (
            <div className="text-center">
              <button
                onClick={runBenchmark}
                className="px-10 py-4 rounded-xl font-semibold text-white bg-kwaai-blue hover:bg-blue-500 transition-colors shadow-lg shadow-blue-500/20 text-lg"
              >
                Benchmark this machine
              </button>
              <p className="text-slate-500 text-sm mt-4">
                Runs entirely in your browser — no data sent anywhere.
              </p>
            </div>
          )}

          {/* ── Running ── */}
          {running && (
            <div className="space-y-4">
              <div className="flex items-center justify-between text-sm">
                <span className="text-slate-300">Analysing hardware…</span>
                <span className="font-mono text-kwaai-blue">{progress}%</span>
              </div>
              <ProgressBar value={progress} color="#3B82F6" />
              <p className="text-xs text-slate-500 text-center">
                Running matrix operations to estimate inference throughput
              </p>
            </div>
          )}

          {/* ── Error ── */}
          {error && (
            <div className="text-center text-kwaai-red">
              <p className="mb-4">Benchmark error: {error}</p>
              <button onClick={runBenchmark} className="text-kwaai-blue underline text-sm">
                Try again
              </button>
            </div>
          )}

          {/* ── Results ── */}
          {results && !running && (
            <div className="space-y-6">
              {/* Hardware metrics */}
              <div className="space-y-4">
                <MetricRow
                  label="GPU"
                  value={`~${results.tokens_per_sec} tokens/sec (${results.method.toUpperCase()})`}
                  pct={Math.min(100, results.tokens_per_sec / 50 * 100)}
                  color="#3B82F6"
                />
                <MetricRow
                  label="Storage"
                  value={`${results.storage_gb} GB available`}
                  pct={Math.min(100, results.storage_gb / 2000 * 100)}
                  color="#10B981"
                />
                <MetricRow
                  label="CPU"
                  value={`${results.cpu_cores} cores`}
                  pct={Math.min(100, results.cpu_cores / 16 * 100)}
                  color="#8B5CF6"
                />
              </div>

              {/* ── External storage ── */}
              <div className="border-t border-slate-800 pt-5">
                <div className="flex items-center justify-between mb-3">
                  <div>
                    <p className="text-sm font-medium text-white">External storage</p>
                    <p className="text-xs text-slate-500 mt-0.5">
                      Pledge space from an external drive to increase earnings
                    </p>
                  </div>
                  {fspSupported ? (
                    <button
                      onClick={() => {
                        toast('Mac name → Macintosh HD → Volumes → your drive', {
                          icon: '💾',
                          duration: 8000,
                        })
                        addDrive()
                      }}
                      disabled={requesting}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm border border-kwaai-green/40 text-kwaai-green hover:bg-kwaai-green/10 transition-colors disabled:opacity-50"
                    >
                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                      </svg>
                      {requesting ? 'Requesting…' : 'Add drive'}
                    </button>
                  ) : (
                    <span className="text-xs text-slate-600">Use Chrome or Edge</span>
                  )}
                </div>

                {drives.length === 0 && (
                  <p className="text-xs text-slate-600 italic">
                    {fspSupported
                      ? 'Click "Add drive" → Mac name → Macintosh HD → Volumes → your drive.'
                      : 'Use Chrome or Edge to enable this feature.'}
                  </p>
                )}

                {drives.map(drive => (
                  <div key={drive.name} className="mt-3 p-3 rounded-lg bg-slate-800/50 border border-slate-700">
                    <div className="flex items-center justify-between mb-2">
                      <div className="flex items-center gap-2 min-w-0">
                        <svg className="w-4 h-4 text-kwaai-green flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M20 7H4a2 2 0 00-2 2v6a2 2 0 002 2h16a2 2 0 002-2V9a2 2 0 00-2-2z" />
                          <circle cx="17" cy="12" r="1" fill="currentColor" />
                        </svg>
                        <span className="text-sm text-slate-300 truncate font-mono">{drive.name}</span>
                      </div>
                      <button
                        onClick={() => removeDrive(drive.name)}
                        className="text-slate-600 hover:text-kwaai-red transition-colors ml-2 flex-shrink-0"
                        title="Remove"
                      >
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                        </svg>
                      </button>
                    </div>
                    <div className="space-y-1.5">
                      <div className="flex items-center justify-between text-xs text-slate-400">
                        <span>Space to contribute</span>
                        <span className="font-mono text-white">{drive.pledgedGb} GB</span>
                      </div>
                      <input
                        type="range"
                        min={50}
                        max={4000}
                        step={50}
                        value={drive.pledgedGb}
                        onChange={e => setPledge(drive.name, Number(e.target.value))}
                        className="w-full accent-kwaai-green"
                      />
                      <div className="flex justify-between text-xs text-slate-600">
                        <span>50 GB</span>
                        <span>4 TB</span>
                      </div>
                    </div>
                  </div>
                ))}
              </div>

              {/* ── Earnings estimate ── */}
              {earnings && (
                <div className="glass border border-kwaai-green/20 bg-kwaai-green/5 p-5 rounded-xl text-center">
                  <div className="text-2xl font-bold text-kwaai-green mb-1">
                    ${earnings.low} – ${earnings.high}
                    <span className="text-base font-normal text-slate-400">/mo</span>
                  </div>
                  <p className="text-slate-400 text-sm">
                    Estimated earnings based on current network demand
                    {totalExternalGb > 0 && (
                      <span className="text-kwaai-green/80"> · includes {totalExternalGb} GB external storage</span>
                    )}
                  </p>
                </div>
              )}

              <div className="text-center">
                <a
                  href="#install"
                  className="inline-block px-8 py-3 rounded-xl font-semibold text-white bg-kwaai-green hover:bg-emerald-500 transition-colors"
                >
                  Join the network →
                </a>
              </div>

              <button
                onClick={runBenchmark}
                className="w-full text-slate-500 text-sm hover:text-slate-300 transition-colors"
              >
                Run again
              </button>
            </div>
          )}
        </div>
      </div>
    </section>
  )
}

function MetricRow({
  label,
  value,
  pct,
  color,
}: {
  label: string
  value: string
  pct: number
  color: string
}) {
  return (
    <div>
      <div className="flex justify-between text-sm mb-1">
        <span className="text-slate-400 w-16">{label}</span>
        <span className="text-white font-mono text-xs">{value}</span>
      </div>
      <ProgressBar value={pct} color={color} />
    </div>
  )
}
