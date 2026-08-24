#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'

const repository = 'https://github.com/castlabs/electron-releases.git'
const stable = /^v(\d+)\.(\d+)\.(\d+)\+wvcus$/
const packagePath = 'sidecar/package.json'
const lockPath = 'sidecar/package-lock.json'
const manifestPath = 'packaging/flatpak/io.github.Jamelade.Jamelade.yml'

function run(command, args) {
  return execFileSync(command, args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  }).trim()
}

function triplet(tag) {
  const match = stable.exec(tag)
  return match ? match.slice(1).map(Number) : null
}

function compare(left, right) {
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index]
  }
  return 0
}

function latestStable() {
  const lines = run('git', [
    'ls-remote',
    '--tags',
    repository,
    'refs/tags/v*+wvcus',
  ]).split(/\r?\n/)
  const releases = lines
    .map((line) => line.trim().split(/\s+/))
    .filter(([, ref]) => ref && !ref.endsWith('^{}'))
    .map(([commit, ref]) => {
      const tag = ref.replace('refs/tags/', '')
      return { commit, tag, version: triplet(tag) }
    })
    .filter((release) => release.version)
    .sort((left, right) => compare(left.version, right.version))
  const latest = releases.at(-1)
  if (!latest) throw new Error('no stable castLabs WVCUS release was found')
  return latest
}

async function releaseAsset(tag) {
  const headers = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'Jamelade-Electron-update-check',
    'X-GitHub-Api-Version': '2022-11-28',
  }
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  const response = await fetch(
    `https://api.github.com/repos/castlabs/electron-releases/releases/tags/${encodeURIComponent(tag)}`,
    { headers },
  )
  if (!response.ok) throw new Error(`GitHub release query failed (${response.status})`)
  const release = await response.json()
  const name = `electron-${tag}-linux-x64.zip`
  const asset = release.assets?.find((candidate) => candidate.name === name)
  if (!asset || !asset.browser_download_url || !asset.digest?.startsWith('sha256:')) {
    throw new Error(`release ${tag} has no checksummed ${name} asset`)
  }
  return {
    sha256: asset.digest.slice('sha256:'.length),
    url: asset.browser_download_url,
  }
}

const release = latestStable()
const asset = await releaseAsset(release.tag)

const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'))
if (!packageJson.devDependencies?.electron) {
  throw new Error('sidecar/package.json has no Electron dependency')
}
const electronSpec =
  `git+https://github.com/castlabs/electron-releases.git#${release.commit}`
packageJson.devDependencies.electron = electronSpec
fs.writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`)

// Refresh transitive metadata without executing dependency scripts, then
// normalize npm's GitHub SSH spelling back to a public HTTPS pin. This keeps
// clean builders independent of SSH configuration while preserving npm's
// dependency-tree update.
run('npm', [
  'install',
  '--package-lock-only',
  '--ignore-scripts',
  '--no-audit',
  '--no-fund',
  '--prefix',
  'sidecar',
])
const packageLock = JSON.parse(fs.readFileSync(lockPath, 'utf8'))
const rootPackage = packageLock.packages?.['']
const electronPackage = packageLock.packages?.['node_modules/electron']
if (!rootPackage?.devDependencies || !electronPackage) {
  throw new Error('sidecar/package-lock.json has no Electron package entries')
}
rootPackage.devDependencies.electron = electronSpec
electronPackage.version = release.tag.slice(1)
electronPackage.resolved = electronSpec
fs.writeFileSync(lockPath, `${JSON.stringify(packageLock, null, 2)}\n`)

let manifest = fs.readFileSync(manifestPath, 'utf8')
const sourcePattern = /(^\s*url:\s*)https:\/\/github\.com\/castlabs\/electron-releases\/releases\/download\/[^\n]+electron-v[^\n]+-linux-x64\.zip\n(^\s*sha256:\s*)[a-f0-9]{64}$/m
if (!sourcePattern.test(manifest)) {
  throw new Error('Flatpak Electron source block was not found')
}
manifest = manifest.replace(sourcePattern, `$1${asset.url}\n$2${asset.sha256}`)
fs.writeFileSync(manifestPath, manifest)

console.log(`castLabs Electron target: ${release.tag}`)
