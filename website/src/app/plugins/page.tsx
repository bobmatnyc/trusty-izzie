import Link from 'next/link'
import type { Metadata } from 'next'
import { ArrowLeft, MessageSquare, Users, Calendar, Puzzle } from 'lucide-react'

export const metadata: Metadata = {
  title: 'Plugins — Izzie',
  description:
    'Extend Izzie with community plugins that add new tools and data sources. Browse built-in plugins and discover community contributions.',
}

export default function PluginsPage() {
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
      <div className="absolute top-1/4 -left-32 w-96 h-96 bg-blue-400/20 rounded-full blur-3xl" />
      <div className="absolute bottom-1/4 -right-32 w-96 h-96 bg-indigo-400/20 rounded-full blur-3xl" />

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

        {/* Hero */}
        <section className="text-center mb-16">
          <div className="inline-flex items-center justify-center w-20 h-20 mb-6 bg-gradient-to-br from-blue-600 to-indigo-600 rounded-2xl shadow-lg shadow-blue-500/25">
            <Puzzle className="w-10 h-10 text-white" />
          </div>
          <h1 className="text-4xl md:text-5xl font-bold tracking-tight text-slate-900 mb-4">
            Izzie Plugins
          </h1>
          <p className="text-xl text-slate-600 max-w-2xl mx-auto leading-relaxed">
            Community plugins extend Izzie with new tools and data sources.
            All plugins run locally on your Mac.
          </p>
        </section>

        {/* Built-in plugins */}
        <section className="mb-16">
          <h2 className="text-2xl font-bold text-slate-900 mb-6">Built-in</h2>
          <div className="grid sm:grid-cols-2 gap-4">
            <PluginCard
              icon={<MessageSquare className="w-6 h-6" />}
              iconBg="bg-blue-100 text-blue-600"
              name="iMessage"
              description="Search and query your local iMessage history."
              badge="Built-in"
            />
            <PluginCard
              icon={<MessageSquare className="w-6 h-6" />}
              iconBg="bg-green-100 text-green-600"
              name="WhatsApp"
              description="Read WhatsApp exports stored on your Mac."
              badge="Built-in"
            />
            <PluginCard
              icon={<Users className="w-6 h-6" />}
              iconBg="bg-indigo-100 text-indigo-600"
              name="Contacts"
              description="Access and search your macOS Contacts."
              badge="Built-in"
            />
            <PluginCard
              icon={<Calendar className="w-6 h-6" />}
              iconBg="bg-amber-100 text-amber-600"
              name="Google Calendar"
              description="Query your Google Calendar events and schedule."
              badge="Built-in"
            />
          </div>
        </section>

        {/* Community plugins */}
        <section className="mb-16">
          <h2 className="text-2xl font-bold text-slate-900 mb-6">Community</h2>
          <div className="bg-white/60 backdrop-blur-sm rounded-2xl border border-slate-200/60 p-12 shadow-sm text-center">
            <div className="inline-flex items-center justify-center w-16 h-16 bg-slate-100 rounded-2xl mb-4 text-slate-400">
              <Puzzle className="w-8 h-8" />
            </div>
            <h3 className="text-lg font-semibold text-slate-800 mb-2">No community plugins yet</h3>
            <p className="text-slate-500 mb-6 max-w-md mx-auto">
              Be the first to publish a plugin for Izzie. The plugin SDK lets you add any
              tool or data source that runs locally on macOS.
            </p>
            <a
              href="https://github.com/bobmatnyc/trusty-izzie"
              className="inline-flex items-center gap-2 px-5 py-2.5 bg-gradient-to-br from-blue-600 to-indigo-600 text-white font-medium rounded-xl hover:from-blue-500 hover:to-indigo-500 transition-all duration-200"
            >
              Publish a plugin &rarr;
            </a>
          </div>
        </section>

        {/* Footer */}
        <footer className="pt-8 border-t border-slate-200">
          <div className="flex flex-wrap justify-between items-center gap-4">
            <div className="flex flex-wrap gap-4 text-sm text-slate-500">
              <Link href="/" className="hover:text-slate-700 transition-colors">Home</Link>
              <span className="text-slate-300">|</span>
              <Link href="/about" className="hover:text-slate-700 transition-colors">About</Link>
              <span className="text-slate-300">|</span>
              <Link href="/privacy" className="hover:text-slate-700 transition-colors">Privacy</Link>
            </div>
          </div>
        </footer>
      </div>
    </main>
  )
}

function PluginCard({
  icon,
  iconBg,
  name,
  description,
  badge,
}: {
  icon: React.ReactNode
  iconBg: string
  name: string
  description: string
  badge: string
}) {
  return (
    <div className="bg-white/60 backdrop-blur-sm rounded-2xl border border-slate-200/60 p-5 shadow-sm hover:shadow-md transition-shadow">
      <div className="flex items-start gap-4">
        <div className={`flex items-center justify-center w-11 h-11 rounded-xl flex-shrink-0 ${iconBg}`}>
          {icon}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <h3 className="font-semibold text-slate-900">{name}</h3>
            <span className="text-xs font-medium px-2 py-0.5 bg-slate-100 text-slate-500 rounded-full">
              {badge}
            </span>
          </div>
          <p className="text-sm text-slate-600">{description}</p>
        </div>
      </div>
    </div>
  )
}
