Vergence is installed, and nothing is enabled yet (Void's way).

Run this once, as root; it says what each step changes and asks first:

    sudo vergence-setup

It enables seatd, vdm (the login screen and lock), the system bus and polkitd
for drives, Wi-Fi where there is a card and a system log, and adds you to the
seat's group. Then log out and back in, or reboot.

If Vergence was also installed from a checkout (tools/install.sh, under
/usr/local), remove that copy first: tools/install.sh --uninstall
