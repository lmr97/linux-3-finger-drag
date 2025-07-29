# Three Finger Drag for Wayland/KDE
This program provides three-finger-drag support computers with touchpads running in Wayland sessions (notably KDE Plasma 6). It only depends on `libinbut` and `uinput`, so the program should run in any desktop environment that has `libinput` installed, regardless of whether it uses X11 or Wayland.

## Tested on...

OS | Version | Desktop Enviroment | Verified
---|---|---|---
**Kubuntu** | 24.10 | KDE Plasma 6 | ✅ <sup>1</sup>
**Arch** | (kernel 6.15.4) | KDE | ✅

---
<sup>1</sup> Developed on this setup.

## Contents

- [What is three-finger dragging?](#what-is-three-finger-dragging)
- [Automated installation](#automated-installation)
- [Manual Installation](#manual-installation)
  - [Step 1 — Install `libinput` dev library](#1-install-the-libinput-dev-library)
  - [Step 2 — Clone the repo](#2-clone-the-repository)
  - [Step 3 — Update permissions](#3-update-permissions)
    - [Step 3.1 — For `uinput`](#31-for-uinput)
    - [Step 3.2 — For `libinput`](#32-for-libinput)
  - [Step 4 — Build with Cargo](#4-build-with-cargo)
  - [Step 5 — Install to `/usr/bin`](#5-install-into-usrbin)
  - [Step 6 — Reboot](#6-reboot)
  - [Step 7 — Add to KDE Autostart](#7-add-program-to-autostart-kde-only)
  - [Step 7b — Set up as `systemd` user unit](#7b-add-program-to-systemd-works-distro-and-desktop-agnostic)
- [Configuration](#configuration)
  - [Set up](#Set-up-configuration)
  - [acceleration](#acceleration-float)
  - [dragEndDelay](#dragenddelay-int)
  - [logFile](#logfile-string)
  - [logLevel](#loglevel-string)
  - [minMotion](#minmotion-float)
  - [responseTime](#responsetime-int)
- [How it works](#how-it-works)
- [Troubleshooting and tips](#troubleshooting-and-tips)
  - [``error: linking with `cc` failed: exit status: 1``](#error-linking-with-cc-failed-exit-status-1-during-compilation)
  - [Changing 3-finger gestures to 4-finger gestures](#changing-3-finger-gestures-to-4-finger-gestures)
    - [For libinput gestures](#for-libinput-gestures-if-needed)
    - [For other extensions](#for-other-extensionsprograms-like-wzmach)


## What is three-finger dragging?

Three-finger dragging is a feature originally for trackpads on Mac devices: instead of holding down the left click on the pad to drag, you can simply rest three fingers on the trackpad to start a mouse hold, and move the fingers together to continue the drag in whatever direction you move them in. In short, it interprets three fingers on the trackpad as a mouse-down input, and motion with three fingers afterwards for mouse movement. It can be quite handy, as it will save your hand some effort for moving windows around and highlighting text. 

Here is [an example](https://www.youtube.com/watch?v=-Fy6imaiHWE) of three-finger dragging in action on a MacBook.

## Automated installation

The included `install.sh` installs the program as a systemd user unit (other inits are not yet supported). It also updates the `libinput-gestures` config files (if you have that installed) so that all 3-finger gestures become 4-finger gestures. 

It requires the following to run properly: 
* **Root permissions**
* A working Rust installation (see [Rust's install guide](https://www.rust-lang.org/tools/install))
* `libinput`'s development library (see Step 1 below for install details)

It will also ask to reboot your system afterward, which is required to update permissions for `libinput` and `uinput`.

You can execute the install script with the following:

```
sudo bash install.sh
```

## Manual installation

### 1. Install the `libinput` dev library

If you are using GNOME, KDE Plasma, or an Xorg-based desktop environment, you likely already have `libinput`'s dev package installed (it's a dependency of those environments). To make sure, try installing it; your package manager will tell you if it is. Here are the commands for some common distros:

#### Debian-based
```
sudo apt install libinput-dev
```

#### Red Hat / Fedora
```
sudo dnf install libinput-devel
```

#### Arch
```
sudo pacman -S libinput   # included in main package
```

#### openSUSE
```
sudo zypper install libinput-devel
```


### 2. Clone the repository
```
git clone https://github.com/lmr97/linux-3-finger-drag.git
cd linux-3-finger-drag
```

### 3. Update permissions

This programs reads cursor events from your trackpad (using `/dev/input/event0`), and writes to `/dev/uinput`, so it requires an adjustment of permissions to accomplish both. 

#### 3.1 For `uinput`
For more info about what's being done here, see [this section](https://wiki.archlinux.org/title/Udev#Allowing_regular_users_to_use_devices) of the ArchWiki article on `udev`. 
You may need to create the folder `rules.d` in `/etc/udev`.

<u>**For Arch users**</u>: You will need to set the `uinput` kernel module to load on boot, if you haven't already, line so:
```
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf 2&>/dev/null
```

Now you can the udev rules to your system:
```
sudo cp ./60-uinput.rules /etc/udev/rules.d
```

#### 3.2 For `libinput`

Simply add yourself to the the user group "input":
```
sudo gpasswd --add <your username> input
```

### 4. Build with Cargo
```
cargo build --release
```

**For my Rustacean users**: A simple `cargo install --path .` will work in place of this step and the next one. Just be sure to substitute `~/.cargo/bin/` for `/usr/bin/` where it appears below and in the provided systemd user unit file (`three-finger-drag.service`), of course.

### 5. Install into `/usr/bin`
Once you've got it working, copy it into `/usr/bin` for ease and consistency of access:

```
sudo cp --preserve=ownership ./target/release/linux-3-finger-drag /usr/bin
```

*Note*: the `--preserve=ownership` option is included so the executable is not run as root, but as you, the user. This keeps the program from being too privileged on your system.

### 6. Reboot

A reboot is required to update all the permissions needed for the program to run. You can also do a soft reboot, which will serve the same purpose, with `systemctl soft-reboot` (for systemd users, of course). 

### 7. Add program to Autostart (KDE only)
This is a part of the graphical interface. You can find the Autostart menu in System Settings > Autostart (near the bottom). Once there, click the "+ Add..." button in the upper right of the window, and select "Add Application" from the dropdown menu. Then, in text bar in the window that pops up, paste
```
/usr/bin/linux-3-finger-drag
```
and click OK. 

Now select the program in the Autostart menu, and press Start in the upper right-hand corner of the window to start using it in the current session. It will automatically start in the next session you log into.

### 7b. Add program to systemd (works distro and desktop agnostic)

Alternatively, you can a [systemd user unit](https://wiki.archlinux.org/title/Systemd/User) in any Linux desktop to start the program on login. To do this, create the local systemd folder if not already created:

```
mkdir -p ~/.config/systemd/user
```

After that, copy the service file in this repo (`three-finger-drag.service`) into the folder:

```
cp three-finger-drag.service ~/.config/systemd/user/
```

Now you just need to enable and start the service:

```
systemctl --user enable --now three-finger-drag.service
```

### You did it! Now you can 3-finger-drag!


## Configuration
This program looks for a JSON config files with the following precedence:

1. `$XDG_CONFIG_HOME/linux-3-finger-drag/3fd-config.json`

2. `~/.config/linux-3-finger-drag/3fd-config.json` (if `$XDG_CONFIG_HOME` isn't set) 

There is an example configuration file included in this repo, `3fd-config.json`, with all fields included and set to default values. 

Below are the fields that can be configured, with the values given here being the defaults. All fields are optional. 
```
{
    acceleration: 1.0,
    dragEndDelay: 0,
    minMotion: 0.2,
    responseTime: 5,
    logFile: "stdout",
    logLevel: "info"
}
```

If the JSON is malformed in the found configuration file, or the file is simply not found, the defaults listed above are loaded instead, and the program continues execution. 

### `acceleration` (float)
This is a speedup multiplier which will be applied to all 3-finger gesture movements. Defaults to 1.0.

### `dragEndDelay` (int)
This is the time (in milliseconds) that the mouse hold will persist for after you lift your fingers (to give you a moment to reposition your fingers). Defaults to 0.

### `logFile` (string)
This allows the user to specify a log file separate from the console/`stdout`. It works best with absolute paths, because `~` or other shell variables are not expanded, but relative filepaths work as well. Note that the program will not create the file if it doesn not exist; in this case, it will simply raise a warning and log to the console. If no file is specified, or the file path is invalid, the program will log to the console. Defaults to `"stdout"`.

### `logLevel` (string)
This allows for the user to control logging verbosity. This can be one of the following values (from least to most verbose):
    
  1. `off`

  2. `error`
  
  3. `warn`
  
  4. `info`
  
  5. `debug`
  
  6. `trace`

For more info on what these levels are intended to capture, see the documentation for [the `enum` to which these values correspond](https://docs.rs/log/0.4.6/log/enum.Level.html). defaults to `"info"`.

### `minMotion` (float)
This is the minimum motion, measured in pixels ([roughly](https://wayland.freedesktop.org/libinput/doc/latest/normalization-of-relative-motion.html)) that the drag gesture has to exceed to cause mouse movement; it's effectively a sensitivity value, but the program becomes less sensitive to mouse input the higher it is. Defaults to 0.2.

### `responseTime` (int)
This is the time (in milliseconds) that the main loop waits before fetching the next batch of events, the inverse of a refresh rate. Defaults to 5.

## How it works
This program uses Rust bindings for libinput to detect three-finger gestures, and translates them into the right events to be written to [`/dev/uinput`](https://www.kernel.org/doc/html/v4.12/input/uinput.html) via a virtual trackpad. This gives the effect of three-finger dragging. This flow of control bypasses the display server layer entirely, which ensures compatability with any desktop environment.

## Troubleshooting and tips

If the fixes here and in the Issues section of the repo don't address your issue, please open a new issue!

### ``error: linking with `cc` failed: exit status: 1`` during compilation

This error arises when some underlying system library can't be found. Cargo produces several "notes" in addition to the error message; look for the final one, or whichever includes a "not found" message. 

If that note includes some mention of `-linput`, then you need to install the development library for `libinput`. (see [Step 1](#1-install-the-libinput-dev-library))

If that text isn't in the note, you may be missing the basic C/C++ developer tools, which are needed to build this program. Rust programs (as I'm using Rust here, anyway) need `gcc` installed on the system to compile. `gcc` is also typically bundled with your distro's "base development" or "build essentials" package, so you can get it that way, too.

### Changing 3-finger gestures to 4-finger gestures

#### For GNOME users

GNOME users will need to install the Window Gestures Shell Extension. Once installed, you'll be able to change the finger number for swipe gestures from your settings. You can get it from either the [GNOME Extensions website](https://extensions.gnome.org/extension/6343/window-gestures/) or the [GitHub repository](https://github.com/amarullz/windowgestures). Once installed, disable all three finger gestures. 

#### For `libinput-gestures` (if needed)

If you haven't installed `libinput-gestures`, you can skip to the next step. 

If you have, though, modify the config file `/etc/libinput-gestures.conf` or `~/.config/libinput-gestures.conf`. 
Add 4 in the finger_count column to convert 3 finger swipes to 4 finger swipes, to prevent confusion for the desktop environment and frustration for yourself.

change
``` 
gesture swipe up     xdotool key super+Page_Down 
```
to
```
gesture swipe up  4  xdotool key super+Page_Down
```
(The only difference is the 4 before "xdotool").

#### For other extensions/programs (like [wzmach](https://github.com/maurges/wzmach))

The process is essentially the same: there is typically a configuration file somewhere that includes the number of fingers for swipe gestures, and if there are any responding to 3-finger swipes, increase the finger count to 4. Consult your program's documentation for the specifics.