import type { Metadata } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'Izzie — Local-first AI Assistant',
  description:
    'Izzie is a local-first AI assistant for Mac. No cloud, no subscriptions. Chat via Telegram.',
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
