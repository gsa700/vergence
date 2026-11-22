---
name: Something is wrong
about: A bug, a crash, something drawn or behaving wrongly
labels: bug
---

**What happened, and what you expected instead**


**How to make it happen again** (steps, if you know them)


**Your machine**
- System: Void Linux / FreeBSD (version)
- Installed from: the package (`xbps-query -p pkgver vergence`) / a checkout (`git rev-parse --short HEAD`)
- GPU(s) and driver: (Settings > About, or `vergence-hw`)
- Screens and their scale, if it is about drawing or placement:

**The server's log** around the time it happened: `~/.local/state/vergence/server.log`
(for the login screen: `/var/lib/vdm/greeter.log`; vdm itself logs to syslog,
on Void `/var/log/socklog`). Paste the lines between three backticks, and read
them first: they name your screens and programs.
