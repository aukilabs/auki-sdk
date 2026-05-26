# auki-domain Binding Surface

This file is the binding coverage contract for `auki-domain`. Every required
operation listed here must have a matching `// binding-surface: ...` marker in
the crate tests before the implementation phase that activates the test.

## Native UniFFI Required

- Cluster lifecycle.
- Manager admission.
- Membership inspection.
- Participant info.
- Domain time and clock estimates.
- Diagnostics.
- Catalog and registry providers.
- Catalog and registry fetches.
- Byte streams.

## Browser JavaScript Required

- Membership validation helpers.
- Manager election helpers.
- Domain DTO validation helpers.
- JavaScript domain client facade over `auki-network` browser transport.
