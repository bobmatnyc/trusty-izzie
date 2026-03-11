import Link from 'next/link'
import { Github, Download, BookOpen, Lock, MessageCircle, Brain, ArrowRight } from 'lucide-react'

const GITHUB_URL = 'https://github.com/bobmatnyc/trusty-izzie'
const RELEASES_URL = 'https://github.com/bobmatnyc/trusty-izzie/releases/latest'

export default function HomePage() {
  return (
    <div className="min-h-screen bg-white text-slate-900">

      {/* ── Nav ── */}
      <header className="sticky top-0 z-50 bg-slate-900 border-b border-slate-800">
        <div className="max-w-5xl mx-auto px-6 h-14 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="text-white font-semibold tracking-tight text-base">Izzie</span>
            <span className="hidden sm:inline text-slate-500 text-xs font-mono border border-slate-700 rounded px-1.5 py-0.5">
              open source
            </span>
          </div>
          <nav className="flex items-center gap-1">
            <a
              href="#docs"
              className="px-3 py-1.5 text-sm text-slate-400 hover:text-white transition-colors rounded-md hover:bg-slate-800"
            >
              Docs
            </a>
            <a
              href={GITHUB_URL}
              className="px-3 py-1.5 text-sm text-slate-400 hover:text-white transition-colors rounded-md hover:bg-slate-800 flex items-center gap-1.5"
            >
              <Github className="w-4 h-4" />
              GitHub
            </a>
            <a
              href={RELEASES_URL}
              className="ml-2 px-3 py-1.5 text-sm bg-white text-slate-900 font-medium rounded-md hover:bg-slate-100 transition-colors flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5" />
              Download
            </a>
          </nav>
        </div>
      </header>

      {/* ── Hero ── */}
      <section className="bg-slate-900 text-white">
        <div className="max-w-5xl mx-auto px-6 py-20 md:py-28">
          <div className="max-w-2xl">
            <div className="mb-5 inline-flex items-center gap-2 text-xs font-mono text-slate-400 border border-slate-700 rounded-full px-3 py-1">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
              MIT licensed · Rust · macOS
            </div>
            <h1 className="text-4xl md:text-5xl font-bold leading-tight tracking-tight mb-5">
              Your AI assistant that<br />actually knows you
            </h1>
            <p className="text-lg text-slate-300 leading-relaxed mb-3">
              Local-first. No cloud. Runs on your Mac. Chat via Telegram.
            </p>
            <p className="text-sm text-slate-500 mb-10">
              Open source &middot; Rust &middot; Your data never leaves your Mac
            </p>
            <div className="flex flex-wrap gap-3">
              <a
                href={RELEASES_URL}
                className="inline-flex items-center gap-2 px-5 py-2.5 bg-white text-slate-900 font-semibold rounded-lg hover:bg-slate-100 transition-colors text-sm"
              >
                <Download className="w-4 h-4" />
                Download for macOS
              </a>
              <a
                href={GITHUB_URL}
                className="inline-flex items-center gap-2 px-5 py-2.5 bg-slate-800 text-white font-semibold rounded-lg hover:bg-slate-700 transition-colors text-sm border border-slate-700"
              >
                <Github className="w-4 h-4" />
                View on GitHub
              </a>
            </div>
          </div>
        </div>
      </section>

      {/* ── How it works ── */}
      <section className="border-b border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-16">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-10">
            How it works
          </h2>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
            <div>
              <div className="w-9 h-9 rounded-lg bg-slate-100 flex items-center justify-center mb-4">
                <Brain className="w-4.5 h-4.5 text-slate-700" />
              </div>
              <h3 className="font-semibold text-slate-900 mb-2">Learns from your email</h3>
              <p className="text-sm text-slate-500 leading-relaxed">
                Reads your sent Gmail (read-only OAuth2) to build a map of your professional relationships, projects, and context. Entities are extracted locally using an LLM and stored in a personal knowledge graph.
              </p>
            </div>
            <div>
              <div className="w-9 h-9 rounded-lg bg-slate-100 flex items-center justify-center mb-4">
                <MessageCircle className="w-4.5 h-4.5 text-slate-700" />
              </div>
              <h3 className="font-semibold text-slate-900 mb-2">Chat anywhere</h3>
              <p className="text-sm text-slate-500 leading-relaxed">
                Talk to Izzie from any device via Telegram. Ask questions about your contacts, calendar, and tasks. Izzie queries your local knowledge base and responds in natural language.
              </p>
            </div>
            <div>
              <div className="w-9 h-9 rounded-lg bg-slate-100 flex items-center justify-center mb-4">
                <Lock className="w-4.5 h-4.5 text-slate-700" />
              </div>
              <h3 className="font-semibold text-slate-900 mb-2">Local-first</h3>
              <p className="text-sm text-slate-500 leading-relaxed">
                All data is stored on your Mac in LanceDB (vectors), Kuzu (graph), and SQLite (auth). Embeddings are generated locally via fastembed (ONNX). Only outbound calls are to OpenRouter for LLM inference and Google APIs for data sync.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* ── What is Izzie ── */}
      <section id="docs" className="border-b border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-16">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-10">
            What is Izzie?
          </h2>
          <div className="grid grid-cols-1 lg:grid-cols-5 gap-12">
            <div className="lg:col-span-3 space-y-8">
              <div>
                <p className="text-slate-600 leading-relaxed mb-4">
                  <strong className="text-slate-900">trusty-izzie</strong> is a headless personal AI assistant that runs entirely on your local machine. It learns from your email and calendar, extracts entities and relationships, and gives you a conversational interface to your own professional context.
                </p>
                <p className="text-slate-600 leading-relaxed">
                  The assistant syncs with Gmail (read-only) and Google Calendar, extracts people, companies, projects, and relationships using an LLM, and builds a personal knowledge graph stored locally. You interact with it via Telegram, CLI, a terminal UI, or a REST API.
                </p>
              </div>

              <div>
                <h3 className="font-semibold text-slate-900 mb-3">What it does</h3>
                <ul className="space-y-2">
                  {[
                    'Syncs with Gmail (OAuth2, read-only) and indexes sent emails',
                    'Extracts people, companies, projects, and relationships using an LLM',
                    'Builds a personal knowledge graph stored in Kuzu (local graph DB)',
                    'Provides hybrid semantic + BM25 search across your memories',
                    'Runs a background daemon for continuous email ingestion',
                    'Exposes a REST API, CLI, and TUI for interaction',
                    'Supports multi-account Google Workspace setups',
                    'Includes a local MCP server for Claude Desktop and Cursor',
                  ].map((item) => (
                    <li key={item} className="flex items-start gap-2.5 text-sm text-slate-600">
                      <ArrowRight className="w-4 h-4 text-slate-400 flex-shrink-0 mt-0.5" />
                      {item}
                    </li>
                  ))}
                </ul>
              </div>
            </div>

            <div className="lg:col-span-2 space-y-6">
              <div>
                <h3 className="font-semibold text-slate-900 mb-3">Architecture</h3>
                <div className="bg-slate-50 rounded-lg border border-slate-200 p-4 text-xs font-mono text-slate-600 leading-relaxed space-y-1">
                  <p className="text-slate-400">// User interfaces</p>
                  <p>trusty-cli &nbsp; trusty-tui &nbsp; trusty-api</p>
                  <p className="text-slate-300">↓</p>
                  <p className="text-slate-400">// Conversation engine</p>
                  <p>trusty-chat (tool dispatch, RAG)</p>
                  <p className="text-slate-300">↓</p>
                  <p className="text-slate-400">// Core services</p>
                  <p>trusty-extractor &nbsp; trusty-memory</p>
                  <p className="text-slate-300">↓</p>
                  <p className="text-slate-400">// Storage</p>
                  <p>LanceDB &nbsp; Kuzu &nbsp; SQLite</p>
                  <p className="text-slate-300">↓</p>
                  <p className="text-slate-400">// Embeddings</p>
                  <p>fastembed (ONNX) &nbsp; tantivy BM25</p>
                </div>
              </div>

              <div>
                <h3 className="font-semibold text-slate-900 mb-3">Key crates</h3>
                <div className="space-y-2">
                  {[
                    { name: 'trusty-models', desc: 'Pure data types' },
                    { name: 'trusty-embeddings', desc: 'Local embedding + BM25' },
                    { name: 'trusty-store', desc: 'LanceDB, Kuzu, SQLite' },
                    { name: 'trusty-extractor', desc: 'LLM-based NER' },
                    { name: 'trusty-chat', desc: 'Conversation engine' },
                    { name: 'trusty-daemon', desc: 'Background sync' },
                  ].map(({ name, desc }) => (
                    <div key={name} className="flex items-baseline gap-2 text-sm">
                      <code className="font-mono text-xs text-slate-700 bg-slate-100 rounded px-1.5 py-0.5 shrink-0">{name}</code>
                      <span className="text-slate-500 text-xs">{desc}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* ── Quick Start ── */}
      <section className="border-b border-slate-200 bg-slate-50">
        <div className="max-w-5xl mx-auto px-6 py-16">
          <div className="flex items-center gap-2 mb-2">
            <BookOpen className="w-4 h-4 text-slate-400" />
            <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400">
              Quick Start
            </h2>
          </div>
          <p className="text-sm text-slate-500 mb-8">
            Prerequisites: Rust 1.75+, a Google Cloud project with Gmail/Calendar APIs enabled, and an OpenRouter API key.
          </p>
          <div className="terminal max-w-2xl">
            <div className="terminal-bar">
              <div className="terminal-dot bg-red-500" />
              <div className="terminal-dot bg-yellow-500" />
              <div className="terminal-dot bg-green-500" />
              <span className="ml-2 text-xs text-slate-500 font-mono">bash</span>
            </div>
            <div className="terminal-body">
              <p><span className="prompt">$</span> git clone https://github.com/bobmatnyc/trusty-izzie</p>
              <p><span className="prompt">$</span> cd trusty-izzie</p>
              <p><span className="prompt">$</span> cp .env.example .env</p>
              <p><span className="comment"># Fill in your Google OAuth and OpenRouter keys</span></p>
              <p><span className="prompt">$</span> cargo build --release</p>
              <p><span className="prompt">$</span> ./scripts/daemon-start.sh</p>
              <p>&nbsp;</p>
              <p><span className="comment"># Authenticate with Google (opens browser)</span></p>
              <p><span className="prompt">$</span> trusty auth google</p>
              <p>&nbsp;</p>
              <p><span className="comment"># Start chatting via CLI</span></p>
              <p><span className="prompt">$</span> trusty chat</p>
            </div>
          </div>
          <div className="mt-6 flex flex-wrap gap-2 text-sm">
            <a href={GITHUB_URL + '#prerequisites'} className="text-slate-500 hover:text-slate-900 underline underline-offset-2 transition-colors">Prerequisites</a>
            <span className="text-slate-300">·</span>
            <a href={GITHUB_URL + '#google-oauth-setup'} className="text-slate-500 hover:text-slate-900 underline underline-offset-2 transition-colors">Google OAuth setup</a>
            <span className="text-slate-300">·</span>
            <a href={GITHUB_URL + '#telegram-integration'} className="text-slate-500 hover:text-slate-900 underline underline-offset-2 transition-colors">Telegram setup</a>
          </div>
        </div>
      </section>

      {/* ── Changelog ── */}
      <section className="border-b border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-16">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-10">
            What&apos;s New
          </h2>
          <div className="space-y-0 max-w-2xl">
            {[
              {
                date: 'Mar 2026',
                title: 'Morning briefing with calendar events and open tasks',
                desc: 'Izzie now queries all connected accounts and tags calendar events and tasks by identity in your morning briefing.',
              },
              {
                date: 'Mar 2026',
                title: 'get_tasks_bulk — fetch all tasks in one call',
                desc: 'Collapses N+1 task calls to a single bulk request per account, dramatically reducing latency for multi-account setups.',
              },
              {
                date: 'Mar 2026',
                title: 'Multi-account support',
                desc: 'Tasks and calendar events now work across all connected Google accounts in a single query.',
              },
              {
                date: 'Mar 2026',
                title: 'Local MCP server',
                desc: 'Use Izzie as an MCP tool source from Claude Desktop or Cursor. Connects via stdio.',
              },
              {
                date: 'Mar 2026',
                title: 'Port conflict fix',
                desc: 'Telegram webhook is now reliable on port 3456 — resolves intermittent startup failures.',
              },
            ].map((item, i) => (
              <div key={i} className="flex gap-6 py-5 border-b border-slate-100 last:border-0">
                <div className="w-20 shrink-0">
                  <span className="text-xs font-mono text-slate-400">{item.date}</span>
                </div>
                <div>
                  <p className="font-medium text-slate-900 text-sm mb-1">{item.title}</p>
                  <p className="text-sm text-slate-500 leading-relaxed">{item.desc}</p>
                </div>
              </div>
            ))}
          </div>
          <div className="mt-8">
            <a href={GITHUB_URL + '/commits/main'} className="inline-flex items-center gap-1.5 text-sm text-slate-500 hover:text-slate-900 transition-colors">
              <Github className="w-4 h-4" />
              Full commit history on GitHub
            </a>
          </div>
        </div>
      </section>

      {/* ── GitHub CTA ── */}
      <section className="bg-slate-900 text-white">
        <div className="max-w-5xl mx-auto px-6 py-16 text-center">
          <h2 className="text-2xl font-bold mb-3">Open source. Self-host your own Izzie.</h2>
          <p className="text-slate-400 mb-8 max-w-md mx-auto text-sm leading-relaxed">
            MIT licensed. Fork it, modify it, run it on your own machine. No accounts, no SaaS, no data leaving your Mac.
          </p>
          <div className="flex flex-wrap justify-center gap-3">
            <a
              href={GITHUB_URL}
              className="inline-flex items-center gap-2 px-5 py-2.5 bg-white text-slate-900 font-semibold rounded-lg hover:bg-slate-100 transition-colors text-sm"
            >
              <Github className="w-4 h-4" />
              bobmatnyc/trusty-izzie
            </a>
            <a
              href={GITHUB_URL + '/issues'}
              className="inline-flex items-center gap-2 px-5 py-2.5 bg-slate-800 text-white font-semibold rounded-lg hover:bg-slate-700 transition-colors text-sm border border-slate-700"
            >
              Open an issue
            </a>
          </div>
        </div>
      </section>

      {/* ── Footer ── */}
      <footer className="border-t border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-8 flex flex-col sm:flex-row items-center justify-between gap-4 text-sm text-slate-400">
          <span>Built with Rust</span>
          <div className="flex flex-wrap justify-center gap-x-5 gap-y-2">
            <a href={GITHUB_URL} className="hover:text-slate-700 transition-colors">GitHub</a>
            <Link href="/privacy" className="hover:text-slate-700 transition-colors">Privacy</Link>
            <Link href="/terms" className="hover:text-slate-700 transition-colors">Terms</Link>
            <a href={GITHUB_URL + '/issues'} className="hover:text-slate-700 transition-colors">Report an issue</a>
          </div>
        </div>
      </footer>

    </div>
  )
}
