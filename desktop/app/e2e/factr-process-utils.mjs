// Process tree helpers for factr packaged smoke tests (darwin + win32).
import { execFileSync } from 'node:child_process'

/** @returns {{ pid: number, ppid: number, comm: string }[]} */
export function listProcesses() {
  if (process.platform === 'win32') {
    const script =
      'Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name | ConvertTo-Json -Compress'
    const raw = execFileSync('powershell.exe', ['-NoProfile', '-Command', script], { encoding: 'utf8' }).trim()
    if (!raw) return []
    const parsed = JSON.parse(raw)
    const rows = Array.isArray(parsed) ? parsed : [parsed]
    return rows
      .filter(row => row && row.ProcessId != null)
      .map(row => ({
        pid: Number(row.ProcessId),
        ppid: Number(row.ParentProcessId),
        comm: String(row.Name || ''),
      }))
  }
  const rows = execFileSync('/bin/ps', ['-axo', 'pid,ppid,comm'], { encoding: 'utf8' })
    .trim()
    .split('\n')
    .slice(1)
    .map(line => line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/))
    .filter(Boolean)
    .map(([, pid, ppid, comm]) => ({ pid: Number(pid), ppid: Number(ppid), comm }))
  return rows
}

/** Descendants of `rootPid` in the process tree. */
export function processTree(rootPid) {
  const rows = listProcesses()
  const owned = new Set([rootPid])
  for (let changed = true; changed;) {
    changed = false
    for (const row of rows) {
      if (owned.has(row.ppid) && !owned.has(row.pid)) {
        owned.add(row.pid)
        changed = true
      }
    }
  }
  return rows.filter(row => owned.has(row.pid))
}

export function isFactrComm(comm) {
  if (!comm) return false
  if (process.platform === 'win32') {
    return /^factr(\.exe)?$/i.test(pathBasename(comm))
  }
  return comm.endsWith('/factr') || comm.endsWith('\\factr')
}

export function isPythonComm(comm) {
  return /python/i.test(comm || '')
}

/** @param {(comm: string) => boolean} matcher */
export function listMatchingProcesses(matcher) {
  return listProcesses().filter(row => matcher(row.comm))
}

export function killProcessHard(pid) {
  if (process.platform === 'win32') {
    execFileSync('taskkill', ['/F', '/PID', String(pid)], { stdio: 'ignore' })
    return
  }
  execFileSync('/bin/kill', ['-9', String(pid)])
}

export function processAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (e) {
    return !(e && (e.code === 'ESRCH' || String(e.message || '').includes('ESRCH')))
  }
}

function pathBasename(name) {
  const parts = name.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || name
}
