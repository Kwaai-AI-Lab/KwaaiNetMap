import { useCallback, useRef } from 'react'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
import ForceGraph2D, { NodeObject, LinkObject } from 'react-force-graph-2d'
import type { NodeEntry } from '../lib/api'
import { tierColor, type TrustTier } from '../theme'

interface GraphNode extends NodeObject {
  id: string
  name: string
  tier: TrustTier
  throughput: number
  blocks: number
}

interface GraphLink extends LinkObject {
  source: string
  target: string
}

function buildGraph(nodes: NodeEntry[]) {
  const graphNodes: GraphNode[] = nodes.map(n => ({
    id: n.peer_id,
    name: n.public_name || n.peer_id.slice(0, 12) + '…',
    tier: n.trust_tier as TrustTier,
    throughput: n.throughput,
    blocks: n.end_block - n.start_block,
  }))

  // Create edges between nodes with overlapping / adjacent block ranges
  const links: GraphLink[] = []
  for (let i = 0; i < nodes.length; i++) {
    for (let j = i + 1; j < nodes.length; j++) {
      const a = nodes[i], b = nodes[j]
      if (a.end_block >= b.start_block && b.end_block >= a.start_block) {
        links.push({ source: a.peer_id, target: b.peer_id })
      }
    }
  }

  return { nodes: graphNodes, links }
}

interface Props {
  nodes: NodeEntry[]
  height?: number
}

export function NetworkGraph({ nodes, height = 400 }: Props) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const fgRef = useRef<any>(null)

  // Sample up to 60 nodes for performance
  const sample = nodes.slice(0, 60)
  const { nodes: graphNodes, links: graphLinks } = buildGraph(sample)

  // Fallback static demo data when no live nodes
  const demoNodes: GraphNode[] = Array.from({ length: 20 }, (_, i) => ({
    id: `demo-${i}`,
    name: `Node ${i + 1}`,
    tier: (['Unknown', 'Known', 'Verified', 'Trusted'] as TrustTier[])[i % 4],
    throughput: Math.random() * 30,
    blocks: 4,
  }))
  const demoLinks: GraphLink[] = demoNodes.slice(0, -1).map((n, i) => ({
    source: n.id,
    target: demoNodes[(i + 1 + Math.floor(Math.random() * 3)) % demoNodes.length].id,
  }))

  const displayNodes = graphNodes.length > 0 ? graphNodes : demoNodes
  const displayLinks = graphNodes.length > 0 ? graphLinks : demoLinks

  const nodeCanvasObject = useCallback((node: NodeObject, ctx: CanvasRenderingContext2D) => {
    const n = node as GraphNode
    const color = tierColor[n.tier]
    const r = Math.max(4, Math.min(12, 4 + n.blocks * 0.3))

    // Outer glow for active nodes
    if (n.throughput > 0) {
      ctx.beginPath()
      ctx.arc(node.x!, node.y!, r + 4, 0, 2 * Math.PI)
      ctx.fillStyle = color + '33'
      ctx.fill()
    }

    ctx.beginPath()
    ctx.arc(node.x!, node.y!, r, 0, 2 * Math.PI)
    ctx.fillStyle = color
    ctx.fill()

    ctx.beginPath()
    ctx.arc(node.x!, node.y!, r, 0, 2 * Math.PI)
    ctx.strokeStyle = color + 'AA'
    ctx.lineWidth = 1
    ctx.stroke()
  }, [])

  return (
    <div className="relative w-full overflow-hidden rounded-xl" style={{ height }}>
      <div
        className="absolute inset-0"
        style={{ background: 'radial-gradient(ellipse at center, #0F1F3D 0%, #0A0F1E 100%)' }}
      />
      <ForceGraph2D
        ref={fgRef}
        graphData={{ nodes: displayNodes, links: displayLinks }}
        nodeCanvasObject={nodeCanvasObject}
        nodeCanvasObjectMode={() => 'replace'}
        linkColor={() => 'rgba(59,130,246,0.15)'}
        linkWidth={1}
        backgroundColor="transparent"
        width={undefined}
        height={height}
        cooldownTicks={80}
        d3AlphaDecay={0.02}
        d3VelocityDecay={0.3}
        enableZoomInteraction={false}
        enablePanInteraction={false}
        nodeLabel={(n) => {
          const node = n as GraphNode
          return `${node.name} · ${node.tier} · ${node.throughput.toFixed(1)} t/s`
        }}
      />
    </div>
  )
}
