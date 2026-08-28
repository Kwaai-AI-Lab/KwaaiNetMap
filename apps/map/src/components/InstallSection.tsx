import { useState } from 'react'
import toast from 'react-hot-toast'

type Platform = 'macos' | 'linux' | 'windows'

function detectPlatform(): Platform {
  const ua = navigator.userAgent
  if (ua.includes('Win')) return 'windows'
  if (ua.includes('Mac')) return 'macos'
  return 'linux'
}

const INSTALL_COMMANDS: Record<Platform, string> = {
  macos: 'curl -fsSL https://raw.githubusercontent.com/Kwaai-AI-Lab/KwaaiNet/main/install.sh | sh',
  linux: 'curl -fsSL https://raw.githubusercontent.com/Kwaai-AI-Lab/KwaaiNet/main/install.sh | sh',
  windows:
    'irm https://raw.githubusercontent.com/Kwaai-AI-Lab/KwaaiNet/main/install.ps1 | iex',
}

const PLATFORM_LABELS: Record<Platform, string> = {
  macos: 'macOS',
  linux: 'Linux',
  windows: 'Windows',
}

export function InstallSection() {
  const [platform, setPlatform] = useState<Platform>(detectPlatform)
  const cmd = INSTALL_COMMANDS[platform]

  const copy = () => {
    navigator.clipboard.writeText(cmd).then(
      () => toast.success('Copied to clipboard'),
      () => toast.error('Copy failed — please copy manually'),
    )
  }

  return (
    <section id="install" className="py-24 px-4">
      <div className="max-w-2xl mx-auto text-center">
        <h2 className="text-3xl md:text-4xl font-bold mb-3">Join the network in 3 steps</h2>
        <p className="text-slate-400 mb-12">
          Your node appears on the map within 60 seconds of starting.
        </p>

        <div className="glass p-8 text-left space-y-8">
          {/* Step 1 */}
          <div>
            <div className="flex items-center gap-3 mb-4">
              <span className="w-7 h-7 rounded-full bg-kwaai-blue/20 border border-kwaai-blue/40 flex items-center justify-center text-kwaai-blue text-sm font-bold">
                1
              </span>
              <span className="font-semibold text-white">Install kwaainet</span>
            </div>

            {/* Platform tabs */}
            <div className="flex gap-2 mb-4">
              {(['macos', 'linux', 'windows'] as Platform[]).map(p => (
                <button
                  key={p}
                  onClick={() => setPlatform(p)}
                  className={`px-4 py-1.5 rounded-lg text-sm font-medium transition-colors ${
                    platform === p
                      ? 'bg-kwaai-blue text-white'
                      : 'text-slate-400 hover:text-white border border-slate-700 hover:border-slate-500'
                  }`}
                >
                  {PLATFORM_LABELS[p]}
                </button>
              ))}
            </div>

            <div className="flex items-center gap-2">
              <code className="flex-1 bg-slate-900 border border-slate-700 rounded-lg px-4 py-3 text-sm font-mono text-kwaai-green break-all">
                {cmd}
              </code>
              <button
                onClick={copy}
                title="Copy to clipboard"
                className="flex-shrink-0 p-3 rounded-lg border border-slate-700 hover:border-kwaai-blue/50 text-slate-400 hover:text-white transition-colors"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <rect x="9" y="9" width="13" height="13" rx="2" />
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                </svg>
              </button>
            </div>
          </div>

          {/* Step 2 */}
          <div>
            <div className="flex items-center gap-3 mb-3">
              <span className="w-7 h-7 rounded-full bg-kwaai-purple/20 border border-kwaai-purple/40 flex items-center justify-center text-kwaai-purple text-sm font-bold">
                2
              </span>
              <span className="font-semibold text-white">Start your node</span>
            </div>
            <code className="block bg-slate-900 border border-slate-700 rounded-lg px-4 py-3 text-sm font-mono text-kwaai-blue">
              kwaainet start --daemon
            </code>
          </div>

          {/* Step 3 */}
          <div>
            <div className="flex items-center gap-3 mb-3">
              <span className="w-7 h-7 rounded-full bg-kwaai-green/20 border border-kwaai-green/40 flex items-center justify-center text-kwaai-green text-sm font-bold">
                3
              </span>
              <span className="font-semibold text-white">Your node appears on the map</span>
            </div>
            <p className="text-slate-400 text-sm pl-10">
              Usually within 60 seconds. Scroll up to see your node pulse live in the network
              graph.
            </p>
          </div>

          {/* CTA */}
          <div className="pt-4 border-t border-slate-800 text-center">
            <a
              href="https://github.com/Kwaai-AI-Lab/KwaaiNet/releases/latest"
              target="_blank"
              rel="noreferrer"
              className="inline-block px-8 py-3 rounded-xl font-semibold text-white bg-kwaai-blue hover:bg-blue-500 transition-colors shadow-lg shadow-blue-500/20"
            >
              Download installer
            </a>
            <p className="text-slate-500 text-xs mt-3">
              macOS · Linux · Windows · Homebrew available
            </p>
          </div>
        </div>
      </div>
    </section>
  )
}
