# Debian 13 (Keystone) deploy notes

- OS: **Debian 13**, not Arch — build with `x86_64-unknown-linux-musl` or inside a Debian 13 container.
- Packages: `pandoc` (required for import). Optional later: `ffmpeg`.
- Data: `/var/lib/diarch` (`diarch.db`, `library/`, `queue/needs_tts/`).
- Port: **8083/tcp** (free on Keystone; 22/53/80/3000/8090/19999 taken).
- Access: open URL `http://keystone:8083` on VPN LAN (same pattern as Pi-hole / DokuWiki).

## First-time setup

```bash
sudo useradd --system --home /var/lib/diarch --shell /usr/sbin/nologin diarch || true
sudo mkdir -p /var/lib/diarch
sudo chown -R diarch:diarch /var/lib/diarch
sudo apt-get install -y pandoc
sudo install -m 755 diarch /usr/local/bin/diarch
sudo cp diarch.service /etc/systemd/system/
# edit Environment= passwords in the unit
sudo systemctl daemon-reload
sudo systemctl enable --now diarch
sudo systemctl status diarch
```
