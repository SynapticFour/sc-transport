# WAN Test mit Fly.io

## Setup (einmalig)
```bash
# Fly CLI installieren
curl -L https://fly.io/install.sh | sh

# Einloggen (kostenloser Account)
fly auth login

# Apps anlegen
fly apps create sct-wan-sender
fly apps create sct-wan-receiver
```

## Test ausführen
```bash
bash scripts/wan_test_flyio.sh
```

## Kosten
Fly.io Free Tier: 3 shared-cpu-1x VMs kostenlos.
2 VMs für ~1 Stunde WAN-Test: $0.00.
