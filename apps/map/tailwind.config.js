/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        'bg-deep': '#0A0F1E',
        'bg-card': '#0F1F3D',
        'kwaai-blue': '#3B82F6',
        'kwaai-green': '#10B981',
        'kwaai-purple': '#8B5CF6',
        'kwaai-amber': '#F59E0B',
        'kwaai-red': '#EF4444',
      },
      fontFamily: {
        mono: ['JetBrains Mono', 'Fira Code', 'monospace'],
      },
    },
  },
  plugins: [],
}
