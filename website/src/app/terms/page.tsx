import Link from 'next/link'
import type { Metadata } from 'next'

export const metadata: Metadata = {
  title: 'Terms of Service - Izzie',
  description: 'Terms of Service for Izzie AI Personal Assistant',
}

export default function TermsOfServicePage() {
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
          <h1 className="text-3xl font-bold text-slate-900 mb-2">Terms of Service</h1>
          <p className="text-slate-500">Last updated: {lastUpdated}</p>
        </div>

        {/* Content */}
        <div className="prose prose-slate max-w-none">
          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">1. Agreement to Terms</h2>
            <p className="text-slate-600 mb-4">
              By downloading or using Izzie (&quot;the Service&quot;), you agree to be bound by these
              Terms of Service. If you do not agree to these terms, please do not use the Service.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">2. Description of Service</h2>
            <p className="text-slate-600 mb-4">
              Izzie is a local-first AI assistant that provides:
            </p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li>AI-powered chat via Telegram interface</li>
              <li>Local access to iMessage, WhatsApp, and Contacts data</li>
              <li>Knowledge graph and semantic search over your local data</li>
              <li>Plugin architecture for extending functionality</li>
              <li>All processing on your local machine — no cloud backend</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">3. User Responsibilities</h2>
            <p className="text-slate-600 mb-4">You agree to:</p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li>Use the Service only for lawful purposes</li>
              <li>Not attempt to reverse engineer or extract proprietary source code</li>
              <li>Not use the Service to violate the privacy of others</li>
              <li>Comply with all applicable laws and regulations</li>
              <li>Comply with Telegram&apos;s Terms of Service when using the bot interface</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">4. Acceptable Use</h2>
            <p className="text-slate-600 mb-4">You may not use the Service to:</p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li>Violate any laws or regulations</li>
              <li>Infringe on intellectual property rights</li>
              <li>Harass, abuse, or harm others</li>
              <li>Generate content that is illegal, harmful, or offensive</li>
              <li>Attempt to bypass security measures</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">5. AI Limitations and Disclaimers</h2>
            <p className="text-slate-600 mb-4">You acknowledge that:</p>
            <ul className="list-disc list-inside text-slate-600 space-y-2">
              <li>AI-generated content may contain errors or inaccuracies</li>
              <li>The Service should not be relied upon for critical decisions without verification</li>
              <li>AI responses are not professional advice (legal, medical, financial, etc.)</li>
              <li>We do not guarantee the accuracy, completeness, or reliability of AI outputs</li>
              <li>You are responsible for reviewing any AI-generated content before use</li>
            </ul>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">6. Intellectual Property</h2>
            <p className="text-slate-600 mb-4">
              The Service, including its source code and documentation, is owned by Izzie and
              protected by applicable intellectual property laws. Open-source components are
              licensed under their respective licenses.
            </p>
            <p className="text-slate-600 mb-4">
              You retain full ownership of all data on your machine. The Service does not claim
              any rights to your personal data.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">7. Privacy</h2>
            <p className="text-slate-600 mb-4">
              Your use of the Service is also governed by our{' '}
              <Link href="/privacy" className="text-blue-600 hover:underline">
                Privacy Policy
              </Link>
              , which describes how we handle your information.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">8. Service Modifications</h2>
            <p className="text-slate-600 mb-4">
              We reserve the right to modify or discontinue the Service at any time, with or
              without notice. We will not be liable for any modification, suspension, or
              discontinuation of the Service.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">9. Disclaimer of Warranties</h2>
            <p className="text-slate-600 mb-4">
              THE SERVICE IS PROVIDED &quot;AS IS&quot; AND &quot;AS AVAILABLE&quot; WITHOUT WARRANTIES
              OF ANY KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO IMPLIED WARRANTIES
              OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">10. Limitation of Liability</h2>
            <p className="text-slate-600 mb-4">
              TO THE MAXIMUM EXTENT PERMITTED BY LAW, WE SHALL NOT BE LIABLE FOR ANY INDIRECT,
              INCIDENTAL, SPECIAL, CONSEQUENTIAL, OR PUNITIVE DAMAGES RESULTING FROM YOUR USE
              OF THE SERVICE.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">11. Governing Law</h2>
            <p className="text-slate-600 mb-4">
              These Terms shall be governed by and construed in accordance with the laws of
              the jurisdiction in which we operate, without regard to conflict of law principles.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">12. Changes to Terms</h2>
            <p className="text-slate-600 mb-4">
              We reserve the right to modify these Terms at any time. We will provide notice of
              significant changes by posting the updated terms on this page and updating the
              &quot;Last updated&quot; date. Continued use of the Service constitutes acceptance.
            </p>
          </section>

          <section className="mb-8">
            <h2 className="text-xl font-semibold text-slate-900 mb-4">13. Contact Us</h2>
            <p className="text-slate-600 mb-4">
              If you have any questions about these Terms of Service, please contact us at:
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
            <Link href="/privacy" className="hover:text-slate-700 transition-colors">Privacy Policy</Link>
            <span className="text-slate-300">|</span>
            <Link href="/" className="hover:text-slate-700 transition-colors">Back to Izzie</Link>
          </div>
        </div>
      </div>
    </main>
  )
}
