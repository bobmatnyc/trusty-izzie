import Link from 'next/link'
import type { Metadata } from 'next'

export const metadata: Metadata = {
  title: 'Izzie — Local-first AI Assistant for Mac',
  description:
    'Izzie is a downloadable Mac app that runs entirely on your machine. Chat via Telegram. No cloud, no subscriptions. Your data never leaves your computer.',
}

/**
 * Landing Page — macOS Download
 */
export default function HomePage() {
  return (
    <main className="min-h-screen flex flex-col items-center justify-center relative overflow-hidden">
      {/* Background */}
      <div className="absolute inset-0 bg-gradient-to-br from-slate-50 via-white to-blue-50" />
      <div
        className="absolute inset-0 opacity-[0.4]"
        style={{
          backgroundImage: `radial-gradient(circle at 1px 1px, rgb(148 163 184 / 0.3) 1px, transparent 0)`,
          backgroundSize: '24px 24px',
        }}
      />

      {/* Decorative orbs */}
      <div className="absolute top-1/4 -left-32 w-96 h-96 bg-blue-400/20 rounded-full blur-3xl" />
      <div className="absolute bottom-1/4 -right-32 w-96 h-96 bg-indigo-400/20 rounded-full blur-3xl" />

      {/* Content */}
      <div className="relative z-10 flex flex-col items-center px-6 py-16 w-full max-w-lg text-center">

        {/* Logo */}
        <div className="inline-flex items-center justify-center w-20 h-20 mb-6 bg-gradient-to-br from-blue-600 to-indigo-600 rounded-2xl shadow-lg shadow-blue-500/25">
          <svg
            className="w-10 h-10 text-white"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            strokeWidth={2}
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M9.75 3.104v5.714a2.25 2.25 0 01-.659 1.591L5 14.5M9.75 3.104c-.251.023-.501.05-.75.082m.75-.082a24.301 24.301 0 014.5 0m0 0v5.714c0 .597.237 1.17.659 1.591L19.8 15.3M14.25 3.104c.251.023.501.05.75.082M19.8 15.3l-1.57.393A9.065 9.065 0 0112 15a9.065 9.065 0 00-6.23-.693L5 14.5m14.8.8l1.402 1.402c1 1 .03 2.998-1.402 2.998H4.2c-1.432 0-2.402-1.998-1.401-2.998L4.2 15.3"
            />
          </svg>
        </div>

        {/* Headline */}
        <h1 className="text-5xl font-bold tracking-tight text-slate-900 mb-4">
          Izzie for Mac
        </h1>
        <p className="text-xl text-slate-600 mb-3 leading-relaxed">
          Local-first AI assistant that runs entirely on your Mac.
        </p>
        <p className="text-base text-slate-500 mb-10">
          Chat via Telegram. Your data never leaves your computer.
        </p>

        {/* Download CTA */}
        <a
          href="https://github.com/bobmatnyc/trusty-izzie/releases/latest"
          className="group inline-flex items-center gap-3 px-8 py-4 bg-gradient-to-br from-blue-600 to-indigo-600 hover:from-blue-500 hover:to-indigo-500 text-white font-semibold rounded-2xl shadow-lg shadow-blue-500/30 hover:shadow-blue-500/40 transition-all duration-200 text-lg mb-4"
        >
          {/* Apple logo */}
          <svg className="w-6 h-6" viewBox="0 0 24 24" fill="currentColor">
            <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.8-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" />
          </svg>
          Download for macOS
        </a>

        <p className="text-sm text-slate-400 mb-12">
          Requires macOS 13 Ventura or later &middot; Free download
        </p>

        {/* Feature highlights */}
        <div className="w-full space-y-3 mb-12 text-left">
          {[
            'Runs entirely on your Mac — no cloud required',
            'Chat with Izzie via Telegram on any device',
            'Reads iMessage, WhatsApp, and Contacts locally',
            'Plugin architecture for custom tools and data sources',
            'Built in Rust for speed and reliability',
          ].map((feature) => (
            <div key={feature} className="flex items-start gap-3 text-sm text-slate-600">
              <svg
                className="w-5 h-5 text-blue-600 flex-shrink-0 mt-0.5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
              <span>{feature}</span>
            </div>
          ))}
        </div>

        {/* Privacy badge */}
        <div className="w-full flex items-center gap-3 px-5 py-4 bg-slate-50 border border-slate-200 rounded-xl mb-10 text-left">
          <svg className="w-8 h-8 text-slate-500 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
          </svg>
          <div>
            <p className="text-sm font-semibold text-slate-800">Private by design</p>
            <p className="text-xs text-slate-500">Your messages, contacts, and insights stay on your Mac. No cloud storage, no subscriptions.</p>
          </div>
        </div>

        {/* Footer links */}
        <div className="flex flex-wrap justify-center gap-x-6 gap-y-2 text-sm text-slate-400">
          <Link href="/about" className="hover:text-slate-600 transition-colors">
            About Izzie
          </Link>
          <Link href="/plugins" className="hover:text-slate-600 transition-colors">
            Plugins
          </Link>
          <Link href="/terms" className="hover:text-slate-600 transition-colors">
            Terms of Service
          </Link>
          <Link href="/privacy" className="hover:text-slate-600 transition-colors">
            Privacy Policy
          </Link>
          <a href="mailto:izzie@matsuoka.com" className="hover:text-slate-600 transition-colors">
            Contact
          </a>
        </div>

      </div>
    </main>
  )
}
