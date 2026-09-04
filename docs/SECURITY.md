# Security model

## Trust boundary

BUTION is designed for computers controlled by the same user on a trusted
private LAN. It is not an internet-facing cluster manager.

- Discovery broadcasts only node metadata and a public key through mDNS.
- Control messages use `Noise_XX_25519_ChaChaPoly_BLAKE2s`.
- First contact requires an explicit TUI decision and displays a short pairing
  code derived from both permanent UUID/public-key tuples.
- Accepted peers are pinned by UUID and static public key, not IP address.
- A key mismatch is rejected instead of silently replacing trust.
- RPC and benchmark servers start only after encrypted pairing.
- Requested bind addresses must exactly match a local, private, non-VPN
  interface. `0.0.0.0`, loopback, public addresses, and unknown addresses fail.
- `llama-server` binds to loopback, so prompts and responses are not exposed to
  the LAN.
- BUTION does not modify firewall rules or request administrator privileges.

## Stored secrets

Noise and Ed25519 private identity material is created with the operating
system's CSPRNG and stored in the per-user application-data directory. Unix file
permissions are restricted to `0600`. Settings contain public trust records and
are written through a replace operation to avoid partial TOML files.

Back up identities only through a trusted encrypted backup. If an identity is
lost or intentionally regenerated, verify the remote machine and re-pair it;
never approve an unexpected key rotation.

## Network policy

Firewall rules should be scoped to the private LAN and the four documented
ports. The RPC port is not a general authenticated service; BUTION's security
comes from opening it only after an authenticated control request and binding it
to the selected private interface. Close BUTION when the cluster is not in use.

The direct llama.cpp RPC data channel follows upstream llama.cpp's security
properties. Do not route it across an untrusted network. Use a separately
administered secure tunnel if the two hosts are not on the same trusted LAN.
