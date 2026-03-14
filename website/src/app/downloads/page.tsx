'use client'

import { useEffect, useState } from 'react'
import Link from 'next/link'
import Image from 'next/image'
import { Github, Download, ArrowRight } from 'lucide-react'

const GITHUB_URL = 'https://github.com/bobmatnyc/trusty-izzie'
const RELEASES_URL = 'https://github.com/bobmatnyc/trusty-izzie/releases'
const RELEASES_API = 'https://api.github.com/repos/bobmatnyc/trusty-izzie/releases/latest'
const FALLBACK_VERSION = 'v0.1.8'

function buildDmgUrl(version: string): string {
  const bare = version.replace('v', '')
  return `${RELEASES_URL}/download/${version}/Izzie_${bare}_universal.dmg`
}

function buildTarballUrl(version: string, target: string): string {
  return `${RELEASES_URL}/download/${version}/trusty-izzie-${version}-${target}.tar.gz`
}

interface CliDownload {
  label: string
  arch: string
  target: string
  platform: string
}

const CLI_DOWNLOADS: CliDownload[] = [
  {
    label: 'macOS Apple Silicon',
    arch: 'ARM64 (M1/M2/M3)',
    target: 'aarch64-apple-darwin',
    platform: 'macOS 12+',
  },
  {
    label: 'macOS Intel',
    arch: 'x86_64',
    target: 'x86_64-apple-darwin',
    platform: 'macOS 12+',
  },
  {
    label: 'Linux x86_64',
    arch: 'x86_64 (musl)',
    target: 'x86_64-unknown-linux-musl',
    platform: 'Linux',
  },
]

export default function DownloadsPage() {
  const [version, setVersion] = useState<string>(FALLBACK_VERSION)
  const [loading, setLoading] = useState<boolean>(true)

  useEffect(() => {
    fetch(RELEASES_API)
      .then((r) => r.json())
      .then((data: { tag_name?: string }) => {
        if (data.tag_name) setVersion(data.tag_name)
      })
      .catch(() => {})
      .finally(() => setLoading(false))
  }, [])

  const dmgUrl = buildDmgUrl(version)
  const dmgFilename = `Izzie_${version.replace('v', '')}_universal.dmg`

  return (
    <div className="min-h-screen bg-white text-slate-900">

      {/* Nav */}
      <header className="sticky top-0 z-50 bg-slate-900 border-b border-slate-800">
        <div className="max-w-5xl mx-auto px-6 h-14 flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <Link href="/" className="flex items-center gap-2.5">
              <Image src="/favicon.png" alt="Izzie" width={28} height={28} className="rounded-md" />
              <span className="text-white font-semibold tracking-tight text-base">Izzie</span>
            </Link>
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
            <Link
              href="/downloads"
              className="ml-2 px-3 py-1.5 text-sm bg-white text-slate-900 font-medium rounded-md hover:bg-slate-100 transition-colors flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5" />
              Download
            </Link>
          </nav>
        </div>
      </header>

      {/* Hero */}
      <section className="bg-slate-900 text-white">
        <div className="max-w-5xl mx-auto px-6 py-16 md:py-20">
          <div className="max-w-2xl">
            <div className="flex items-center gap-3 mb-6">
              <span
                className={`inline-flex items-center gap-2 text-xs font-mono border rounded-full px-3 py-1 transition-colors ${
                  loading
                    ? 'text-slate-500 border-slate-700'
                    : 'text-emerald-400 border-slate-700'
                }`}
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full ${loading ? 'bg-slate-500' : 'bg-emerald-400'}`}
                />
                {loading ? 'Checking latest release...' : version}
              </span>
            </div>
            <h1 className="text-4xl md:text-5xl font-bold leading-tight tracking-tight mb-4">
              Download Izzie
            </h1>
            <p className="text-lg text-slate-300 leading-relaxed mb-8">
              Your private AI assistant, running locally on your Mac.
            </p>
            <a
              href={dmgUrl}
              className="inline-flex items-center gap-2 px-6 py-3 bg-blue-600 hover:bg-blue-500 text-white font-semibold rounded-lg transition-colors text-sm"
            >
              <Download className="w-4 h-4" />
              Download for macOS
            </a>
            <p className="text-xs text-slate-500 mt-3">
              {dmgFilename} &middot; Universal Binary &middot; macOS 12+
            </p>
          </div>
        </div>
      </section>

      {/* Primary download card */}
      <section className="border-b border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-12">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-8">
            Recommended
          </h2>
          <div className="bg-slate-800 rounded-xl border border-slate-700 p-6 max-w-xl">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h3 className="text-white font-semibold text-base mb-1">macOS App</h3>
                <p className="text-slate-400 text-sm">Recommended for most users</p>
              </div>
              <span className="text-xs font-mono text-slate-400 border border-slate-600 rounded px-2 py-0.5">
                {loading ? '...' : version}
              </span>
            </div>

            <div className="flex flex-wrap gap-2 mb-5">
              <span className="text-xs font-mono text-slate-300 bg-slate-700 rounded px-2 py-0.5">
                Apple Silicon + Intel
              </span>
              <span className="text-xs font-mono text-slate-300 bg-slate-700 rounded px-2 py-0.5">
                macOS 12+
              </span>
              <span className="text-xs font-mono text-slate-300 bg-slate-700 rounded px-2 py-0.5">
                ~90 MB
              </span>
            </div>

            <a
              href={dmgUrl}
              className="inline-flex items-center gap-2 px-5 py-2.5 bg-blue-600 hover:bg-blue-500 text-white font-semibold rounded-lg transition-colors text-sm w-full justify-center mb-4"
            >
              <Download className="w-4 h-4" />
              Download {dmgFilename}
            </a>

            {/* TODO: Remove this notice once Apple notarization is complete */}
            <p className="text-xs text-amber-400/80 bg-amber-400/5 border border-amber-400/20 rounded-lg px-3 py-2">
              First launch: right-click the app icon and choose Open to bypass Gatekeeper. Apple notarization is in progress.
            </p>
          </div>
        </div>
      </section>

      {/* CLI / Advanced downloads */}
      <section className="border-b border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-12">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-2">
            CLI &amp; Advanced Downloads
          </h2>
          <p className="text-sm text-slate-500 mb-8">
            Command-line tools for developers and headless environments.
          </p>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            {CLI_DOWNLOADS.map((dl) => {
              const filename = `trusty-izzie-${version}-${dl.target}.tar.gz`
              const url = buildTarballUrl(version, dl.target)
              return (
                <a
                  key={dl.target}
                  href={url}
                  className="bg-slate-800 rounded-xl border border-slate-700 p-5 hover:border-slate-500 transition-colors group"
                >
                  <div className="mb-3">
                    <span className="text-xs font-mono text-slate-400 bg-slate-700 rounded px-2 py-0.5">
                      CLI Tools
                    </span>
                  </div>
                  <h3 className="text-white font-medium text-sm mb-1 group-hover:text-blue-400 transition-colors">
                    {dl.label}
                  </h3>
                  <p className="text-slate-400 text-xs mb-3">{dl.arch}</p>
                  <p className="text-slate-500 text-xs mb-4">{dl.platform}</p>
                  <div className="flex items-center gap-1.5 text-slate-400 text-xs group-hover:text-slate-300 transition-colors">
                    <Download className="w-3 h-3 shrink-0" />
                    <span className="font-mono truncate">{filename}</span>
                  </div>
                </a>
              )
            })}
          </div>
        </div>
      </section>

      {/* System requirements */}
      <section className="border-b border-slate-200 bg-slate-50">
        <div className="max-w-5xl mx-auto px-6 py-12">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-8">
            System Requirements
          </h2>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-8 max-w-2xl">
            <div>
              <h3 className="font-semibold text-slate-900 text-sm mb-3">macOS App</h3>
              <ul className="space-y-2">
                {[
                  'macOS 12.0 Monterey or later',
                  'Apple Silicon (M1+) or Intel Mac',
                  '~200 MB disk space (includes AI model files)',
                  'Internet connection required for AI features',
                ].map((req) => (
                  <li key={req} className="flex items-start gap-2 text-sm text-slate-600">
                    <ArrowRight className="w-3.5 h-3.5 text-slate-400 shrink-0 mt-0.5" />
                    {req}
                  </li>
                ))}
              </ul>
            </div>
            <div>
              <h3 className="font-semibold text-slate-900 text-sm mb-3">CLI Tools</h3>
              <ul className="space-y-2">
                {[
                  'macOS 12+ or Linux (glibc or musl)',
                  'x86_64 or ARM64 (Apple Silicon)',
                  'Terminal access',
                  'Rust 1.75+ (if building from source)',
                ].map((req) => (
                  <li key={req} className="flex items-start gap-2 text-sm text-slate-600">
                    <ArrowRight className="w-3.5 h-3.5 text-slate-400 shrink-0 mt-0.5" />
                    {req}
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      </section>

      {/* Installation instructions */}
      <section className="border-b border-slate-200">
        <div className="max-w-5xl mx-auto px-6 py-12">
          <h2 className="text-xs font-semibold uppercase tracking-widest text-slate-400 mb-8">
            Installation
          </h2>
          <div className="max-w-xl space-y-4">
            {[
              {
                step: '1',
                title: 'Download the DMG',
                desc: 'Click the download button above to get the universal DMG for both Apple Silicon and Intel Macs.',
              },
              {
                step: '2',
                title: 'Open and drag to Applications',
                desc: 'Open the downloaded DMG file, then drag the Izzie icon to your Applications folder.',
              },
              {
                step: '3',
                title: 'First launch: right-click to open',
                desc: 'On first launch, right-click (or Control-click) the Izzie icon in Applications and choose Open. This bypasses macOS Gatekeeper until notarization is complete.',
              },
              {
                step: '4',
                title: 'Follow the setup wizard',
                desc: 'Connect your Gmail account with OAuth2 and configure your preferred chat interface (Telegram, CLI, or TUI).',
              },
            ].map((item) => (
              <div key={item.step} className="flex gap-4">
                <div className="w-7 h-7 rounded-full bg-slate-100 flex items-center justify-center shrink-0 mt-0.5">
                  <span className="text-xs font-semibold text-slate-500">{item.step}</span>
                </div>
                <div>
                  <p className="font-medium text-slate-900 text-sm mb-1">{item.title}</p>
                  <p className="text-sm text-slate-500 leading-relaxed">{item.desc}</p>
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* All releases */}
      <section className="bg-slate-900 text-white">
        <div className="max-w-5xl mx-auto px-6 py-12 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
          <div>
            <h2 className="font-semibold text-white mb-1">All releases &amp; changelog</h2>
            <p className="text-sm text-slate-400">
              View release notes, older versions, and checksums on GitHub.
            </p>
          </div>
          <a
            href={RELEASES_URL}
            className="inline-flex items-center gap-2 px-5 py-2.5 bg-slate-800 text-white font-semibold rounded-lg hover:bg-slate-700 transition-colors text-sm border border-slate-700 shrink-0"
          >
            <Github className="w-4 h-4" />
            See all versions
          </a>
        </div>
      </section>

      {/* Footer */}
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
