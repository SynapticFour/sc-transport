# WAN Test mit AWS EC2 (AWS Activate Credits)

## Voraussetzungen
1. AWS Activate Credits beantragen:
   https://aws.amazon.com/startups/credits
   (Founders-Paket: $1.000, kein Org-ID nötig)
2. AWS CLI konfigurieren: `aws configure`
3. EC2 Key Pair `sct-wan-key` und Security Group `sg-sct-wan` (Port 22 + 9410) in
   `eu-central-1` und `us-east-1` anlegen.

## Instanzen starten
```bash
bash infra/aws/start_wan_test.sh
```

## Kosten (aus Credits)
2× t4g.small (ARM, 2 vCPU, 2 GB) je $0.0168/h:
1 Stunde Test = ~$0.034 — aus $1.000 Credits: faktisch kostenlos.

## Aufräumen
```bash
bash infra/aws/teardown.sh
```
