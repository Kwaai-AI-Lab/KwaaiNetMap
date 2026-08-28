import { useEffect, useRef } from 'react'
import * as d3 from 'd3'
import type { NodeEntry } from '../lib/api'
import { tierColor, type TrustTier } from '../theme'

interface Props {
  nodes: NodeEntry[]
}

interface D3Node extends d3.SimulationNodeDatum {
  id: string
  tier: TrustTier
  throughput: number
  label: string
}

interface D3Link extends d3.SimulationLinkDatum<D3Node> {
  source: string | D3Node
  target: string | D3Node
}

const TIERS: TrustTier[] = ['Unknown', 'Known', 'Verified', 'Trusted']

const VC_CARDS = [
  {
    title: 'SummitAttendeeVC',
    tier: 'Known' as TrustTier,
    desc: 'Issued at Kwaai Summit events. Establishes community membership.',
  },
  {
    title: 'VerifiedNodeVC',
    tier: 'Verified' as TrustTier,
    desc: 'On-chain node identity verification. Enables Verified tier.',
  },
  {
    title: 'FiduciaryPledgeVC',
    tier: 'Trusted' as TrustTier,
    desc: 'Highest trust tier. Commit to network fiduciary responsibilities.',
  },
  {
    title: 'PeerEndorsementVC',
    tier: 'Verified' as TrustTier,
    desc: 'Direct peer-to-peer endorsements. Builds the trust graph.',
  },
]

export function TrustGraphSection({ nodes }: Props) {
  const svgRef = useRef<SVGSVGElement>(null)

  // Build sample graph from live nodes or use demo data
  useEffect(() => {
    const el = svgRef.current
    if (!el) return

    const W = el.clientWidth || 600
    const H = 300

    // Sample up to 20 nodes
    const sample = (nodes.length > 0 ? nodes : generateDemoNodes()).slice(0, 20)
    const d3Nodes: D3Node[] = sample.map(n => ({
      id: n.peer_id,
      tier: n.trust_tier as TrustTier,
      throughput: n.throughput,
      label: n.public_name || n.peer_id.slice(0, 8),
    }))
    const d3Links: D3Link[] = d3Nodes.slice(0, -1).map((n, i) => ({
      source: n.id,
      target: d3Nodes[(i + 1 + (i % 3)) % d3Nodes.length].id,
    }))

    d3.select(el).selectAll('*').remove()

    const svg = d3.select(el)
      .attr('width', W)
      .attr('height', H)

    svg.append('defs').append('radialGradient')
      .attr('id', 'bg-grad')
      .selectAll('stop')
      .data([
        { offset: '0%', color: '#0F1F3D' },
        { offset: '100%', color: '#0A0F1E' },
      ])
      .join('stop')
      .attr('offset', d => d.offset)
      .attr('stop-color', d => d.color)

    svg.append('rect').attr('width', W).attr('height', H).attr('fill', 'url(#bg-grad)')

    const sim = d3.forceSimulation<D3Node>(d3Nodes)
      .force('link', d3.forceLink<D3Node, D3Link>(d3Links).id(d => d.id).distance(60))
      .force('charge', d3.forceManyBody().strength(-80))
      .force('center', d3.forceCenter(W / 2, H / 2))
      .force('collide', d3.forceCollide(20))

    const link = svg.append('g')
      .selectAll('line')
      .data(d3Links)
      .join('line')
      .attr('stroke', 'rgba(59,130,246,0.2)')
      .attr('stroke-width', 1)

    const node = svg.append('g')
      .selectAll('circle')
      .data(d3Nodes)
      .join('circle')
      .attr('r', d => 6 + Math.min(8, d.throughput * 0.3))
      .attr('fill', d => tierColor[d.tier])
      .attr('opacity', 0.9)

    sim.on('tick', () => {
      link
        .attr('x1', d => (d.source as D3Node).x!)
        .attr('y1', d => (d.source as D3Node).y!)
        .attr('x2', d => (d.target as D3Node).x!)
        .attr('y2', d => (d.target as D3Node).y!)
      node
        .attr('cx', d => d.x!)
        .attr('cy', d => d.y!)
    })

    return () => { sim.stop() }
  }, [nodes])

  return (
    <section id="network" className="py-24 px-4">
      <div className="max-w-5xl mx-auto">
        <div className="text-center mb-12">
          <h2 className="text-3xl md:text-4xl font-bold mb-3">Trust, verifiable by design</h2>
          <p className="text-slate-400 max-w-xl mx-auto">
            KwaaiNet implements the Linux Foundation ToIP 4-layer trust stack. Every node's
            reputation is derived from on-chain Verifiable Credentials — no central authority.
          </p>
        </div>

        {/* D3 force graph */}
        <div className="glass overflow-hidden mb-8">
          <svg ref={svgRef} className="w-full" style={{ height: 300 }} />
        </div>

        {/* Tier legend */}
        <div className="flex flex-wrap justify-center gap-6 mb-12">
          {TIERS.map(tier => (
            <div key={tier} className="flex items-center gap-2">
              <span
                className="inline-block w-3 h-3 rounded-full"
                style={{ background: tierColor[tier] }}
              />
              <span className="text-sm text-slate-300">{tier}</span>
            </div>
          ))}
        </div>

        {/* VC cards */}
        <div className="grid md:grid-cols-2 gap-4">
          {VC_CARDS.map(card => (
            <div key={card.title} className="glass p-5">
              <div className="flex items-start gap-3">
                <span
                  className="mt-1 inline-block w-3 h-3 rounded-full flex-shrink-0"
                  style={{ background: tierColor[card.tier] }}
                />
                <div>
                  <div className="font-mono text-sm text-kwaai-blue mb-1">{card.title}</div>
                  <p className="text-slate-400 text-sm">{card.desc}</p>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}

function generateDemoNodes(): NodeEntry[] {
  return Array.from({ length: 20 }, (_, i) => ({
    peer_id: `demo-${i}`,
    trust_tier: (['Unknown', 'Known', 'Verified', 'Trusted'] as TrustTier[])[i % 4],
    start_block: i * 4,
    end_block: i * 4 + 3,
    throughput: Math.random() * 25,
    public_name: `node-${i + 1}`,
    version: 'kwaai-0.3.22',
    vpk: i % 3 === 0,
    last_seen: new Date().toISOString(),
  }))
}
