// Design tokens from KwaaiNet banner.svg / brand guidelines

export const colors = {
  bgDeep: '#0A0F1E',
  bgCard: '#0F1F3D',
  blue: '#3B82F6',
  green: '#10B981',
  purple: '#8B5CF6',
  amber: '#F59E0B',
  red: '#EF4444',
  text: '#E2E8F0',
  textMuted: '#94A3B8',
  border: 'rgba(59,130,246,0.15)',
} as const

export type TrustTier = 'Unknown' | 'Known' | 'Verified' | 'Trusted'

export const tierColor: Record<TrustTier, string> = {
  Unknown: '#EF4444',
  Known: '#F59E0B',
  Verified: '#8B5CF6',
  Trusted: '#10B981',
}

export const tierColorHex = tierColor
