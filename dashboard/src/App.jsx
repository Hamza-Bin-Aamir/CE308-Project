import { useEffect, useMemo, useState } from 'react'
import { fetchDashboardData, sendCommand } from './api'

const refreshMs = 4000

function StatCard({ label, value, hint, accent }) {
  return (
    <div className={`stat-card stat-${accent}`}>
      <div className="stat-label">{label}</div>
      <div className="stat-value">{value}</div>
      <div className="stat-hint">{hint}</div>
    </div>
  )
}

function formatTime(ms) {
  if (!ms) return 'n/a'
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

function formatAgo(ms) {
  if (!ms) return 'n/a'
  const delta = Date.now() - ms
  if (delta < 60_000) return `${Math.max(1, Math.round(delta / 1000))}s ago`
  if (delta < 3_600_000) return `${Math.round(delta / 60_000)}m ago`
  return `${Math.round(delta / 3_600_000)}h ago`
}

function Badge({ children, tone = 'secondary' }) {
  return <span className={`badge text-bg-${tone}`}>{children}</span>
}

export default function App() {
  const [dashboard, setDashboard] = useState({ summary: {}, drones: [], alerts: [], commands: [] })
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [selectedDrone, setSelectedDrone] = useState('')
  const [altitude, setAltitude] = useState('100')
  const [mission, setMission] = useState('patrol_perimeter')
  const [sending, setSending] = useState('')
  const [lastUpdated, setLastUpdated] = useState(null)

  async function loadData() {
    try {
      const data = await fetchDashboardData()
      setDashboard(data)
      setError('')
      setLastUpdated(Date.now())
    } catch (err) {
      setError(err.message || 'Failed to load dashboard data')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadData()
    const timer = setInterval(loadData, refreshMs)
    return () => clearInterval(timer)
  }, [])

  useEffect(() => {
    if (dashboard.drones.length > 0 && !dashboard.drones.some((drone) => drone.device_id === selectedDrone)) {
      setSelectedDrone(dashboard.drones[0].device_id)
    }
  }, [dashboard.drones, selectedDrone])

  const selectedDroneInfo = useMemo(
    () => dashboard.drones.find((drone) => drone.device_id === selectedDrone),
    [dashboard.drones, selectedDrone],
  )

  async function handleQuickCommand(command) {
    if (!selectedDrone) return
    setSending(command)
    try {
      await sendCommand(selectedDrone, command)
      await loadData()
    } catch (err) {
      setError(err.message || 'Failed to send command')
    } finally {
      setSending('')
    }
  }

  async function handleSetAltitude(event) {
    event.preventDefault()
    await handleQuickCommand({ type: 'set_altitude', altitude_m: Number(altitude) })
  }

  async function handleSetMission(event) {
    event.preventDefault()
    await handleQuickCommand({ type: 'set_mission', mission })
  }

  const summary = dashboard.summary || {}
  const drones = dashboard.drones || []
  const alerts = dashboard.alerts || []
  const commands = dashboard.commands || []

  return (
    <div className="dashboard-shell">
      <div className="dashboard-glow dashboard-glow-left" />
      <div className="dashboard-glow dashboard-glow-right" />

      <main className="container-fluid py-4 py-lg-5 position-relative">
        <div className="dashboard-hero mb-4 mb-lg-5">
          <div>
            <div className="eyebrow">CE308 Fleet Control</div>
            <h1 className="display-5 fw-bold mb-2">Drone operations dashboard</h1>
            <p className="lead mb-0 text-white-75">
              Live fleet status, alert monitoring, and direct command control in one view.
            </p>
          </div>
          <div className="hero-metadata">
            <div className="meta-chip">
              <span className="meta-label">Last refresh</span>
              <span className="meta-value">{lastUpdated ? formatAgo(lastUpdated) : 'waiting...'}</span>
            </div>
            <div className="meta-chip">
              <span className="meta-label">Selected drone</span>
              <span className="meta-value">{selectedDrone || 'none'}</span>
            </div>
          </div>
        </div>

        {error ? <div className="alert alert-warning border-0 shadow-sm">{error}</div> : null}
        {loading ? <div className="alert alert-info border-0 shadow-sm">Loading fleet data...</div> : null}

        <section className="row g-3 g-xl-4 mb-4 mb-xl-5">
          <div className="col-6 col-xl-3"><StatCard label="Total drones" value={summary.total_drones ?? 0} hint="Tracked from latest telemetry" accent="cyan" /></div>
          <div className="col-6 col-xl-3"><StatCard label="Online" value={summary.online_drones ?? 0} hint="Telemetry seen within 10s" accent="green" /></div>
          <div className="col-6 col-xl-3"><StatCard label="Offline" value={summary.offline_drones ?? 0} hint="Not recently reporting" accent="amber" /></div>
          <div className="col-6 col-xl-3"><StatCard label="Recent alerts" value={summary.recent_alerts ?? alerts.length} hint="Current alert feed size" accent="pink" /></div>
        </section>

        <section className="row g-4 align-items-start">
          <div className="col-12 col-xl-8">
            <div className="panel-card">
              <div className="panel-header">
                <div>
                  <h2 className="h4 mb-1">Fleet status</h2>
                  <p className="mb-0 text-white-50">Battery, altitude, online state, and latest command per drone.</p>
                </div>
              </div>

              <div className="table-responsive">
                <table className="table table-dark table-hover align-middle mb-0 fleet-table">
                  <thead>
                    <tr>
                      <th>Drone</th>
                      <th>State</th>
                      <th>Battery</th>
                      <th>Altitude</th>
                      <th>Last seen</th>
                      <th>Latest command</th>
                    </tr>
                  </thead>
                  <tbody>
                    {drones.map((drone) => (
                      <tr key={drone.device_id} onClick={() => setSelectedDrone(drone.device_id)} role="button">
                        <td>
                          <div className="fw-semibold">{drone.device_id}</div>
                          <div className="small text-white-50">{formatTime(drone.last_seen_ms)}</div>
                        </td>
                        <td>{drone.online ? <Badge tone="success">online</Badge> : <Badge tone="secondary">offline</Badge>}</td>
                        <td>{drone.battery_voltage_v?.toFixed?.(2) ?? 'n/a'} V</td>
                        <td>{drone.altitude_m?.toFixed?.(1) ?? 'n/a'} m</td>
                        <td>{formatAgo(drone.last_seen_ms)}</td>
                        <td>
                          {drone.latest_command ? (
                            <div>
                              <div className="fw-semibold text-capitalize">{drone.latest_command.command_kind.replace(/_/g, ' ')}</div>
                              <div className="small text-white-50 text-capitalize">{drone.latest_command.status}</div>
                            </div>
                          ) : (
                            <span className="text-white-50">Monitoring</span>
                          )}
                        </td>
                      </tr>
                    ))}
                    {!drones.length ? (
                      <tr><td colSpan="6" className="text-center text-white-50 py-4">No drones found yet.</td></tr>
                    ) : null}
                  </tbody>
                </table>
              </div>
            </div>
          </div>

          <div className="col-12 col-xl-4">
            <div className="panel-card h-100">
              <div className="panel-header">
                <div>
                  <h2 className="h4 mb-1">Command drone</h2>
                  <p className="mb-0 text-white-50">Target a drone and send operational commands.</p>
                </div>
              </div>

              <label className="form-label text-white-75">Target drone</label>
              <select className="form-select form-select-lg mb-3" value={selectedDrone} onChange={(event) => setSelectedDrone(event.target.value)}>
                {drones.map((drone) => <option key={drone.device_id} value={drone.device_id}>{drone.device_id}</option>)}
              </select>

              <div className="d-grid gap-2 mb-3 command-grid">
                <button className="btn btn-outline-info" disabled={!selectedDrone || sending} onClick={() => handleQuickCommand({ type: 'ping' })}>Ping</button>
                <button className="btn btn-outline-success" disabled={!selectedDrone || sending} onClick={() => handleQuickCommand({ type: 'start' })}>Start</button>
                <button className="btn btn-outline-warning" disabled={!selectedDrone || sending} onClick={() => handleQuickCommand({ type: 'stop' })}>Stop</button>
                <button className="btn btn-outline-light" disabled={!selectedDrone || sending} onClick={() => handleQuickCommand({ type: 'return_home' })}>Return Home</button>
              </div>

              <form className="command-form mb-3" onSubmit={handleSetAltitude}>
                <label className="form-label text-white-75">Set altitude</label>
                <div className="input-group">
                  <input type="number" className="form-control" value={altitude} onChange={(event) => setAltitude(event.target.value)} min="0" step="1" />
                  <span className="input-group-text">m</span>
                  <button className="btn btn-primary" type="submit" disabled={!selectedDrone || sending}>Send</button>
                </div>
              </form>

              <form className="command-form" onSubmit={handleSetMission}>
                <label className="form-label text-white-75">Set mission</label>
                <div className="input-group">
                  <input type="text" className="form-control" value={mission} onChange={(event) => setMission(event.target.value)} />
                  <button className="btn btn-primary" type="submit" disabled={!selectedDrone || sending}>Send</button>
                </div>
              </form>

              <div className="selected-drone-card mt-4">
                <div className="small text-white-50">Selected drone snapshot</div>
                <div className="fw-semibold fs-5">{selectedDroneInfo?.device_id || 'No drone selected'}</div>
                <div className="small text-white-75">Status: {selectedDroneInfo?.online ? 'online' : 'offline'}</div>
                <div className="small text-white-75">Battery: {selectedDroneInfo?.battery_voltage_v?.toFixed?.(2) ?? 'n/a'} V</div>
                <div className="small text-white-75">Altitude: {selectedDroneInfo?.altitude_m?.toFixed?.(1) ?? 'n/a'} m</div>
                <div className="small text-white-75">Latest command: {selectedDroneInfo?.latest_command?.command_kind?.replace(/_/g, ' ') ?? 'none'}</div>
              </div>
            </div>
          </div>
        </section>

        <section className="row g-4 mt-1">
          <div className="col-12 col-lg-6">
            <div className="panel-card h-100">
              <div className="panel-header">
                <div>
                  <h2 className="h4 mb-1">Recent alerts</h2>
                  <p className="mb-0 text-white-50">Emitted rule violations from the telemetry pipeline.</p>
                </div>
              </div>

              <div className="timeline-list">
                {alerts.map((alert) => (
                  <div key={alert.message_id} className="timeline-item">
                    <div className="timeline-dot timeline-dot-alert" />
                    <div>
                      <div className="d-flex flex-wrap gap-2 align-items-center mb-1">
                        <span className="fw-semibold">{alert.device_id}</span>
                        <Badge tone="danger">{alert.rule_kind}</Badge>
                        <span className="small text-white-50">{formatAgo(alert.timestamp_ms)}</span>
                      </div>
                      <div className="text-white-75">{alert.message}</div>
                    </div>
                  </div>
                ))}
                {!alerts.length ? <div className="text-white-50">No alerts have been recorded yet.</div> : null}
              </div>
            </div>
          </div>

          <div className="col-12 col-lg-6">
            <div className="panel-card h-100">
              <div className="panel-header">
                <div>
                  <h2 className="h4 mb-1">Command timeline</h2>
                  <p className="mb-0 text-white-50">Queued commands and device acknowledgments.</p>
                </div>
              </div>

              <div className="timeline-list">
                {commands.map((command) => (
                  <div key={command.message_id} className="timeline-item">
                    <div className={`timeline-dot ${command.status === 'acknowledged' ? 'timeline-dot-ok' : 'timeline-dot-command'}`} />
                    <div>
                      <div className="d-flex flex-wrap gap-2 align-items-center mb-1">
                        <span className="fw-semibold">{command.device_id}</span>
                        <Badge tone={command.status === 'acknowledged' ? 'success' : command.status === 'failed' ? 'danger' : 'info'}>{command.status}</Badge>
                        <span className="small text-white-50 text-capitalize">{command.command_kind.replace(/_/g, ' ')}</span>
                        <span className="small text-white-50">{formatAgo(command.timestamp_ms)}</span>
                      </div>
                      <div className="text-white-75">{command.detail || 'No extra detail recorded.'}</div>
                    </div>
                  </div>
                ))}
                {!commands.length ? <div className="text-white-50">No commands have been sent yet.</div> : null}
              </div>
            </div>
          </div>
        </section>
      </main>
    </div>
  )
}