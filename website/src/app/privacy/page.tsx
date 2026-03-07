import Link from 'next/link'
import type { Metadata } from 'next'

export const metadata: Metadata = {
  title: 'Privacy Policy - Izzie',
  description: 'Privacy Policy for Izzie AI Personal Assistant',
}

export default function PrivacyPolicyPage() {
  const lastUpdated = 'January 23, 2026'

  return (
    <main className="min-h-screen relative overflow-hidden">
      {/* Background */}
      <div className="absolute inset-0 bg-gradient-to-br from-slate-50 via-white to-blue-50" />
      <div
        className="absolute inset-0 opacity-[0.4]"
        style={{
          backgroundImage: `radial-gradient(circle at 1px 1px, rgb(148 163 184 / 0.3) 1px, transparent 0)`,
          backgroundSize: '24px 24px',
        }}
      />

      {/* Content */}
      <div className="relative z-10 max-w-3xl mx-auto px-6 py-12">
        {/* Header */}
        <div className="mb-8">
          <Link
            href="/"
            className="inline-flex items-center gap-2 text-slate-600 hover:text-slate-900 transition-colors mb-6"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
            </svg>
            Back to Izzie
          </Link>
          <h1 className="text-3xl font-bold text-slate-900 mb-2">Privacy Policy</h1>
          <p className="text-slate-500">Last updated: {lastUpdated}</p>
        </div>

        {/* Content */}
        <div className="prose prose-slate max-w-none">
          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Introduction</h2>
            <p className="text-slate-600 mb-4">
              Izzie (&quot;we&quot;, &quot;our&quot;, or &quot;the Service&quot;) is a local-first AI
              assistant that runs entirely on your Mac. This Privacy Policy explains how we handle
              your information when you use Izzie.
            </p>
            <p className="text-slate-600 mb-4">
              The core principle of Izzie is that <strong>your data never leaves your computer</strong>.
              All processing happens locally on your machine.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Information We Collect</h2>

            <h3 className="text-lg font-medium text-slate-800 mb-2">Local Data Access</h3>
            <p className="text-slate-600 mb-4">
              Izzie reads data from your local Mac including iMessage history, WhatsApp exports,
              and Contacts — all of which remain on your device. This data is processed locally
              and is never transmitted to any external server.
            </p>

            <h3 className="text-lg font-medium text-slate-800 mb-2">Telegram Bot</h3>
            <p className="text-slate-600 mb-4">
              Izzie uses a personal Telegram bot as its interface. Messages sent via Telegram
              are relayed to your local Izzie daemon. Telegram&apos;s own privacy policy applies
              to messages in transit through their platform.
            </p>

            <h3 className="text-lg font-medium text-slate-800 mb-2">AI Processing</h3>
            <p className="text-slate-600 mb-4">
              When you send queries to Izzie, relevant context may be sent to an AI model provider
              (such as Anthropic Claude) to generate responses. Only the minimum necessary
              information is sent to process your query.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Data Storage</h2>
            <p className="text-slate-600 mb-4">
              All data Izzie stores is kept locally on your Mac in its application directory.
              No data is synced to cloud servers. You retain full ownership and control.
            </p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li>All indices and knowledge graphs are stored locally</li>
              <li>Conversation history stays on your machine</li>
              <li>No analytics, telemetry, or usage data is collected</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Third-Party Services</h2>
            <p className="text-slate-600 mb-4">
              Izzie integrates with the following third-party services:
            </p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li><strong>Telegram:</strong> Used as the chat interface. Subject to Telegram&apos;s privacy policy.</li>
              <li><strong>Anthropic / OpenAI:</strong> For AI processing. Only query context is sent, not your raw data stores.</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Your Rights</h2>
            <p className="text-slate-600 mb-4">Because all data is local, you have complete control:</p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li><strong>Delete:</strong> Uninstall Izzie and delete its application data directory</li>
              <li><strong>Inspect:</strong> All stored data is in readable formats in the app directory</li>
              <li><strong>Portability:</strong> Your data is on your Mac — export or migrate it freely</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Changes to This Policy</h2>
            <p className="text-slate-600 mb-4">
              We may update this Privacy Policy from time to time. We will notify you of any
              significant changes by posting the new policy on this page and updating the
              &quot;Last updated&quot; date.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">Contact Us</h2>
            <p className="text-slate-600 mb-4">
              If you have any questions about this Privacy Policy, please contact us at:
            </p>
            <p className="text-slate-600">
              Email:{' '}
              <a href="mailto:izzie@matsuoka.com" className="text-blue-600 hover:underline">
                izzie@matsuoka.com
              </a>
            </p>
          </section>
        </div>

        {/* Footer */}
        <div className="mt-12 pt-8 border-t border-slate-200">
          <div className="flex flex-wrap gap-4 text-sm text-slate-500">
            <Link href="/terms" className="hover:text-slate-700 transition-colors">Terms of Service</Link>
            <span className="text-slate-300">|</span>
            <Link href="/" className="hover:text-slate-700 transition-colors">Back to Izzie</Link>
          </div>
        </div>
      </div>
    </main>
  )
}
