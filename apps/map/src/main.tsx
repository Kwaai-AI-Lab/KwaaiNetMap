import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { Toaster } from 'react-hot-toast'
import './index.css'
import App from './App'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
    <Toaster
      position="bottom-right"
      toastOptions={{
        style: {
          background: '#0F1F3D',
          color: '#E2E8F0',
          border: '1px solid #1E3A5F',
        },
      }}
    />
  </StrictMode>,
)
