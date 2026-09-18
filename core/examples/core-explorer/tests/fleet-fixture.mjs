// DDS b27c080 / DMS 06bd863 wire contracts, also documented in
// docs/reference/fleet.md and Web src/fleet.rs. These are provider responses,
// never fabricated frontend FleetSnapshot objects.
export function fleetResponse({ method, url, headers }, state, { domain, other, config, capability }) {
  const robot = /^\/api\/v1\/domains\/([^/]+)\/robots$/.exec(url.pathname);
  const nodes = url.pathname === '/api/v1/nodes';
  if (!robot && !nodes) return undefined;
  if (method !== 'GET' || headers.authorization !== 'Bearer synthetic-service') return { status: 403, body: {} };
  if (nodes && (url.searchParams.get('org') !== 'all' || url.searchParams.get('staking_status') !== 'all')) return { status: 422, body: {} };
  if (state.fleetDenied === (robot ? 'robots' : 'nodes')) return { status: 403, body: {} };
  const installation = state.fleetMultiple ? other : config.installationId;
  const role = robot ? 'robot' : 'compute';
  const value = { id: config[`${role}Id`], organization_id: config.installationId, name: robot ? 'Demo inspector' : 'Demo text worker', capabilities: [capability(role)], status: state.fleetOffline ? 'offline' : state.fleetUnknown ? 'new-provider-status' : 'online', ...(robot ? { assigned_domain_id: state.fleetWrongDomain ? other : domain, last_seen_at: null, active_lease_expires_at: null } : { mode: 'dedicated' }) };
  if (state.fleetCrossKind === 'both' || state.fleetCrossKind === (robot ? 'compute' : 'robot')) value.capabilities.push(capability(robot ? 'compute' : 'robot'));
  const values = state.fleetEmpty || state.fleetMissing === role || (robot && robot[1] !== domain) ? [] : [value];
  if (state.fleetDuplicate && nodes) values.push({ ...value, id: other, name: 'Second compute candidate' });
  if (state.fleetMultiple && nodes) values.push({ ...value, id: other, name: 'Other installation worker', capabilities: [`/examples/compute-robot/${installation}/compute/v1`] });
  if (state.fleetBroad && nodes && !state.fleetEmpty) values.push({ ...value, id: 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee', name: 'Public <worker>', mode: 'public', capabilities: ['vendor/arbitrary/v9'] });
  for (const machine of values) machine.status = state.fleetPresence?.[machine.id] ?? machine.status;
  return { status: 200, body: { [robot ? 'robots' : 'nodes']: values } };
}
