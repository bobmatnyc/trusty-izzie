import type { Metadata } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'Izzie — Local-first AI Assistant for Mac',
  description:
    'Open-source AI assistant that learns from your Gmail and calendar. Runs entirely on your Mac. Chat via Telegram. Self-host your own Izzie.',
  icons: {
    icon: '/favicon.svg',
    apple: '/favicon.png',
  },
}

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode
}>) {
  return (
    <html lang="en">
      <body className="antialiased">{children}</body>
    </html>
  )
}
