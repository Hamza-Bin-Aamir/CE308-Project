const apiBaseUrl = (import.meta.env.VITE_API_BASE_URL || '').replace(/\/$/, '')

function endpoint(path) {
  return `${apiBaseUrl}${path}`
}

async function requestJson(path, options = {}) {
  const response = await fetch(endpoint(path), {
    headers: {
      'Content-Type': 'application/json',
      ...(options.headers || {}),
    },
    ...options,
  })

  const text = await response.text()
  const payload = text ? JSON.parse(text) : null

  if (!response.ok) {
    throw new Error(payload?.error || `Request failed with ${response.status}`)
  }

  return payload
}

export async function fetchDashboardData() {
  const [summary, drones, alerts, commands] = await Promise.all([
    requestJson('/api/summary'),
    requestJson('/api/drones'),
    requestJson('/api/alerts/recent'),
    requestJson('/api/commands/recent'),
  ])

  return { summary, drones, alerts, commands }
}

export async function sendCommand(deviceId, command) {
  return requestJson(`/api/command/${encodeURIComponent(deviceId)}`, {
    method: 'POST',
    body: JSON.stringify(command),
  })
}
