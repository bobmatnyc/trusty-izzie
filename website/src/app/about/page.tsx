import Link from 'next/link'
import type { Metadata } from 'next'
import {
  MessageSquare,
  Users,
  Puzzle,
  Shield,
  Lock,
  Trash2,
  Search,
  Zap,
  Smartphone,
  Network,
  ArrowLeft,
} from 'lucide-react'

export const metadata: Metadata = {
  title: 'About Izzie — Local-first AI Assistant for Mac',
  description:
    'Learn how Izzie, a private Mac app built in Rust, gives you an AI assistant that lives on your computer with a Telegram interface, plugin architecture, and no cloud dependency.',
}

export default function AboutPage() {
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

      {/* Decorative gradient orbs */}
      <div className="absolute top-1/4 -left-32 w-96 h-96 bg-blue-400/20 rounded-full blur-3xl" />
      <div className="absolute bottom-1/4 -right-32 w-96 h-96 bg-indigo-400/20 rounded-full blur-3xl" />
      <div className="absolute top-3/4 left-1/2 w-64 h-64 bg-purple-400/10 rounded-full blur-3xl" />

      {/* Content */}
      <div className="relative z-10 max-w-4xl mx-auto px-6 py-12">
        {/* Navigation */}
        <div className="mb-8">
          <Link
            href="/"
            className="inline-flex items-center gap-2 text-slate-600 hover:text-slate-900 transition-colors"
          >
            <ArrowLeft className="w-4 h-4" />
            Back to Izzie
          </Link>
        </div>

        {/* Hero Section */}
        <section className="text-center mb-16">
          <div className="inline-flex items-center justify-center w-20 h-20 mb-6 bg-gradient-to-br from-blue-600 to-indigo-600 rounded-2xl shadow-lg shadow-blue-500/25">
            <MessageSquare className="w-10 h-10 text-white" />
          </div>
          <h1 className="text-4xl md:text-5xl font-bold tracking-tight text-slate-900 mb-4">
            About Izzie
          </h1>
          <p className="text-xl text-slate-600 max-w-2xl mx-auto leading-relaxed">
            A local-first AI assistant for Mac — built in Rust, accessed through Telegram,
            and completely private by design
          </p>
        </section>

        {/* Features Section */}
        <section className="space-y-12 mb-16">
          {/* Feature 1: Local-first Architecture */}
          <FeatureCard
            icon={<Zap className="w-6 h-6" />}
            iconBg="bg-blue-100 text-blue-600"
            title="Local-first Architecture"
            description="Izzie runs as a native Rust daemon on your Mac. No cloud, no server, no subscription required."
          >
            <FeatureList
              items={[
                { icon: <Zap className="w-4 h-4" />, text: 'Built in Rust for performance and reliability' },
                { icon: <Shield className="w-4 h-4" />, text: 'All processing happens on your machine' },
                { icon: <Lock className="w-4 h-4" />, text: 'No account required — install and run' },
              ]}
            />
          </FeatureCard>

          {/* Feature 2: Telegram Interface */}
          <FeatureCard
            icon={<Smartphone className="w-6 h-6" />}
            iconBg="bg-indigo-100 text-indigo-600"
            title="Telegram Interface"
            description="Chat with Izzie from any device using Telegram. Your bot runs locally but your interface travels with you."
          >
            <FeatureList
              items={[
                { icon: <MessageSquare className="w-4 h-4" />, text: 'Dedicated private Telegram bot per user' },
                { icon: <Smartphone className="w-4 h-4" />, text: 'Access from phone, tablet, or desktop' },
                { icon: <Search className="w-4 h-4" />, text: 'Natural language queries answered instantly' },
              ]}
            />
          </FeatureCard>

          {/* Feature 3: Built-in Data Sources */}
          <FeatureCard
            icon={<Users className="w-6 h-6" />}
            iconBg="bg-purple-100 text-purple-600"
            title="Built-in Data Sources"
            description="Izzie reads your local data directly — no OAuth, no permissions pop-ups, no middleman."
          >
            <div className="grid sm:grid-cols-2 gap-3">
              <EntityItem icon={<MessageSquare className="w-4 h-4" />} label="iMessage" />
              <EntityItem icon={<MessageSquare className="w-4 h-4" />} label="WhatsApp" />
              <EntityItem icon={<Users className="w-4 h-4" />} label="Contacts" />
              <EntityItem icon={<Network className="w-4 h-4" />} label="Knowledge Graph" />
            </div>
          </FeatureCard>

          {/* Feature 4: Plugin Architecture */}
          <FeatureCard
            icon={<Puzzle className="w-6 h-6" />}
            iconBg="bg-green-100 text-green-600"
            title="Plugin Architecture"
            description="Extend Izzie with community plugins that add new tools and data sources."
          >
            <FeatureList
              items={[
                { icon: <Puzzle className="w-4 h-4" />, text: 'Install community plugins from the registry' },
                { icon: <Zap className="w-4 h-4" />, text: 'Plugins run locally with full privacy' },
                { icon: <Network className="w-4 h-4" />, text: 'Publish your own plugins to share with others' },
              ]}
            />
          </FeatureCard>
        </section>

        {/* Privacy Section */}
        <section className="mb-16">
          <div className="bg-white/60 backdrop-blur-sm rounded-2xl border border-slate-200/60 p-8 shadow-sm">
            <div className="flex items-center gap-3 mb-6">
              <div className="flex items-center justify-center w-12 h-12 bg-slate-100 rounded-xl text-slate-600">
                <Shield className="w-6 h-6" />
              </div>
              <h2 className="text-2xl font-bold text-slate-900">Your Privacy is Non-Negotiable</h2>
            </div>

            <div className="grid sm:grid-cols-3 gap-6">
              <PrivacyItem
                icon={<Lock className="w-5 h-5" />}
                title="No Cloud Storage"
                description="All your data lives on your Mac. Nothing is uploaded to any server — not even anonymized telemetry."
              />
              <PrivacyItem
                icon={<Shield className="w-5 h-5" />}
                title="No Data Sharing"
                description="Your messages, contacts, and AI conversations are never sold or shared with anyone."
              />
              <PrivacyItem
                icon={<Trash2 className="w-5 h-5" />}
                title="Full Control"
                description="Uninstall Izzie and all data is gone. No account to close, no data to request deletion of."
              />
            </div>
          </div>
        </section>

        {/* CTA Section */}
        <section className="text-center mb-16">
          <div className="bg-gradient-to-br from-blue-600 to-indigo-600 rounded-2xl p-8 shadow-lg shadow-blue-500/25">
            <h2 className="text-2xl font-bold text-white mb-3">Download Izzie for Mac</h2>
            <p className="text-blue-100 mb-6">
              Free download. Runs entirely on your Mac. Your data never leaves your computer.
            </p>
            <a
              href="https://github.com/bobmatnyc/trusty-izzie/releases/latest"
              className="inline-flex items-center gap-2 px-6 py-3 bg-white hover:bg-slate-50 text-blue-600 font-semibold rounded-xl shadow-sm transition-all duration-200"
            >
              <svg className="w-5 h-5" viewBox="0 0 24 24" fill="currentColor">
                <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.8-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" />
              </svg>
              Download for macOS
            </a>
          </div>
        </section>

        {/* Footer */}
        <footer className="pt-8 border-t border-slate-200">
          <div className="flex flex-wrap justify-between items-center gap-4">
            <div className="flex flex-wrap gap-4 text-sm text-slate-500">
              <Link href="/" className="hover:text-slate-700 transition-colors">Home</Link>
              <span className="text-slate-300">|</span>
              <Link href="/plugins" className="hover:text-slate-700 transition-colors">Plugins</Link>
              <span className="text-slate-300">|</span>
              <Link href="/terms" className="hover:text-slate-700 transition-colors">Terms of Service</Link>
              <span className="text-slate-300">|</span>
              <Link href="/privacy" className="hover:text-slate-700 transition-colors">Privacy Policy</Link>
            </div>
            <div className="text-sm text-slate-500">
              Contact:{' '}
              <a href="mailto:izzie@matsuoka.com" className="text-blue-600 hover:underline">
                izzie@matsuoka.com
              </a>
            </div>
          </div>
        </footer>
      </div>
    </main>
  )
}

function FeatureCard({
  icon,
  iconBg,
  title,
  description,
  children,
}: {
  icon: React.ReactNode
  iconBg: string
  title: string
  description: string
  children: React.ReactNode
}) {
  return (
    <div className="bg-white/60 backdrop-blur-sm rounded-2xl border border-slate-200/60 p-6 shadow-sm hover:shadow-md transition-shadow">
      <div className="flex items-start gap-4 mb-4">
        <div className={`flex items-center justify-center w-12 h-12 rounded-xl ${iconBg}`}>
          {icon}
        </div>
        <div>
          <h3 className="text-xl font-semibold text-slate-900">{title}</h3>
          <p className="text-slate-600 mt-1">{description}</p>
        </div>
      </div>
      <div className="ml-0 sm:ml-16">{children}</div>
    </div>
  )
}

function FeatureList({ items }: { items: Array<{ icon: React.ReactNode; text: string }> }) {
  return (
    <ul className="space-y-3">
      {items.map((item, index) => (
        <li key={index} className="flex items-start gap-3">
          <span className="flex-shrink-0 mt-0.5 text-slate-400">{item.icon}</span>
          <span className="text-slate-600">{item.text}</span>
        </li>
      ))}
    </ul>
  )
}

function EntityItem({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <div className="flex items-center gap-2 px-3 py-2 bg-slate-50 rounded-lg">
      <span className="text-slate-500">{icon}</span>
      <span className="text-slate-700 font-medium">{label}</span>
    </div>
  )
}

function PrivacyItem({
  icon,
  title,
  description,
}: {
  icon: React.ReactNode
  title: string
  description: string
}) {
  return (
    <div className="text-center sm:text-left">
      <div className="flex justify-center sm:justify-start mb-3">
        <div className="flex items-center justify-center w-10 h-10 bg-slate-100 rounded-lg text-slate-600">
          {icon}
        </div>
      </div>
      <h3 className="font-semibold text-slate-900 mb-1">{title}</h3>
      <p className="text-sm text-slate-600">{description}</p>
    </div>
  )
}
