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
  - [Step 0 — Install `libinput` dev library](#0-install-the-libinput-dev-library)
  - [Step 1 — Clone the repo](#1-clone-the-repository)
  - [Step 2 — Change existing three-finger gestures](#2-change-any-existing-3-finger-gestures-to-4-finger-gestures)
    - [For libinput gestures](#for-libinput-gestures-if-needed)
    - [For other extensions](#for-other-extensionsprograms-like-wzmach)
  - [Step 3 — Update permissions](#3-update-permissions)
    - [Step 3.1 — For `uinput`](#31-for-uinput)
    - [Step 3.2 — For `libinput`](#32-for-libinput)
  - [Step 4 — Build with Cargo](#4-build-with-cargo)
  - [Step 5 — Reboot](#5-reboot)
  - [Step 6 — Verify functionality](#6-verify-functionality)
  - [Step 7 — Install to `/usr/bin`](#7-install-into-usrbin)
  - [Step 8 — Set up configuration](#8-set-up-configuration-file-optional)
  - [Step 9 — Add to KDE Autostart](#9-add-program-to-autostart-kde-only)
  - [Step 9b — Set up as `systemd` user unit](#9b-add-program-to-systemd-works-distro-and-desktop-agnostic)
- [Configuration](#configuration):
  - [acceleration](#acceleration-float)
  - [dragEndDelay](#dragenddelay-int)
  - [minMotion](#minmotion-float)
  - [failFast](#failfast-boolean)
  - [logFile](#logfile-string)
  - [logLevel](#loglevel-string)
- [How it works](#how-it-works)


## What is three-finger dragging?

Three-finger dragging is a feature originally for trackpads on Mac devices: instead of holding down the left click on the pad to drag, you can simply rest three fingers on the trackpad to start a mouse hold, and move the fingers together to continue the drag in whatever direction you move them in. In short, it interprets three fingers on the trackpad as a mouse-down input, and motion with three fingers afterwards for mouse movement. It can be quite handy, as it will save your hand some effort for moving windows around and highlighting text. 

Here is [an example](https://www.youtube.com/watch?v=-Fy6imaiHWE) of three-finger dragging in action on a MacBook.

## Automated installation

The included `install.sh` installs the program as a SystemD user unit (other inits are not yet supported). It also updates the `libinput-gestures` config files (if you have that installed) so that all 3-finger gestures become 4-finger gestures. 

It requires the following to run properly: 
* **Root permissions**
* A working Rust installation (see [Rust's install guide](https://www.rust-lang.org/tools/install))
* `libinput`'s development library (see Step 0 below for install details)

It will also ask to reboot your system afterward, which is required to update permissions for `libinput` and `uinput`.

You can execute the install script with the following:

```
sudo bash install.sh
```

## Getting updates

You can update your version of `linux-3-finger-drag` using the `update.sh` script. 

## Manual installation

### 0. Install the `libinput` dev library

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


### 1. Clone the repository
```
git clone https://github.com/lmr97/linux-3-finger-drag.git
cd linux-3-finger-drag
```

### 2. Change any existing 3-finger gestures to 4-finger gestures

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
sudo gpasswd --add your-username-here input
```

### 4. Build with Cargo
```
cargo build --release
```

### 5. Reboot

A reboot is required to update all the permissions needed for the program to run. You can also do a soft reboot, which will serve the same purpose, with `systemctl soft-reboot` (for SystemD users, of course). 

### 6. Verify functionality
Check to make sure the executable works by running
```
./target/release/linux-3-finger-drag
```

You will see a warning about not being able to find a configuration file. Just ignore that for now, as it will not impact functionality; we'll get to configuration in step 7. 

I've tried to make error messages as informative as I can, with recommended actions included to remedy them, so look to those error messages for help first. But if they don't help, please submit an issue here, with information about your OS, desktop environment, and any error output you get, and I'll get on it as soon as I can. 

### 7. Install into `/usr/bin`
Once you've got it working, copy it into `/usr/bin` for ease and consistency of access:
```
sudo cp ./target/release/linux-3-finger-drag /usr/bin
```

**For my Rustacean friends**: a simple `cargo install --path .` will also work (with the permissions updated from step 3). Just make sure to substitute `~/.cargo/bin/` for `/usr/bin/` where it appears below, of course.

### 8. Set up configuration file (optional)

See the Configuration section below about the included `3fd-config.json` file. If you'd like to configure the behavior of this program, run the following:
```
mkdir ~/.config/linux-3-finger-drag
cp 3fd-config.json ~/.config/linux-3-finger-drag
```
Now you can configure to your heart's content!

### 9. Add program to Autostart (KDE only)
This is a part of the graphical interface. You can find the Autostart menu in System Settings > Autostart (near the bottom). Once there, click the "+ Add..." button in the upper right of the window, and select "Add Application" from the dropdown menu. Then, in text bar in the window that pops up, paste
```
/usr/bin/linux-3-finger-drag
```
and click OK. 

Now select the program in the Autostart menu, and press Start in the upper right-hand corner of the window to start using it in the current session. It will automatically start in the next session you log into.

### 9b. Add program to SystemD (works distro and desktop agnostic)

Alternatively, you can add this program to the autostart in any linux desktop and autostart it via systemd. To do this copy this file create the local systemd folder if not already created:

```
mkdir -p ~/.config/systemd/user
```

After that copy the service file in this repo to the folder you just created (or just the folder if you already have one):

```
cp three-finger-drag.service ~/.config/systemd/user/
```

Now you just need to enable and start the service:

```
systemctl --user enable --now three-finger-drag.service
```

### You did it! Now you can 3-finger-drag!


## Configuration
There is a JSON configuration file, assumed to be at `~/.config/linux-3-finger-drag/3fd-config.json`, which is read into the program at startup. It has the following fields, that have the given values as defaults (`?` indicates an entirely optional field): 
```
{
    acceleration: 1.0,
    dragEndDelay: 0,
    minMotion: 0.2,
    failFast?: false
    logFile?: "stdout"    // this will mean logging to stdout
    logLevel: "info"
}
```

If any of the required values are not present, or the JSON is malformed in the found configuration file, the defaults listed above are loaded instead.

### `acceleration` (float)
(**Required**) This is a speedup multiplier which will be applied to all 3-finger gesture movements. Defaults to 1.0.

### `dragEndDelay` (int)
(**Required**) This is the time (in milliseconds) that the mouse hold will persist for after you lift your fingers (to give you a moment to reposition your fingers). Defaults to 0.

### `minMotion` (float)
(**Required**) This is the minimum motion, measured in pixels ([roughly](https://wayland.freedesktop.org/libinput/doc/latest/normalization-of-relative-motion.html)) that the drag gesture has to exceed to cause mouse movement; it's effectively a sensitivity value, but the program becomes less sensitive to mouse input the higher it is. Defaults to 0.2.

### `failFast` (boolean)
(*Optional*) This indicates whether the program is to exit with an error when the first runtime error is encountered. It will exit with an error if one is encountered during setup regardless of how this option is set. Defaults to `false`.

### `logFile` (string)
(*Optional*) This allows the user to specify a log file separate from the console/`stdout`. It works best with absolute paths, because `~` or other shell variables are not expanded, but relative filepaths work as well. Note that the program will not create the file if it doesn not exist; in this case, it will simply raise a warning and log to the console. If no file is specified, or the file path is invalid, the program will log to the console. Defaults to `"stdout"`.

### `logLevel` (string)
(*Optional*) This allows for the user to control logging verbosity. This can be one of the following values (from least to most verbose):
    1. `off`
    2. `error`
    3. `warn`
    4. `info`
    5. `debug`
    6. `trace`
For more info on what these levels are intended to capture, see the documentation for [the `enum`](https://docs.rs/log/0.4.6/log/enum.Level.html) to which these values correspond. defaults to `"info"`.

## How it works
This program uses Rust bindings for libinput to detect three-finger gestures, and translates them into the right events to be written to [`/dev/uinput`](https://www.kernel.org/doc/html/v4.12/input/uinput.html) via a virtual trackpad. This gives the effect of three-finger dragging. This flow of control bypasses the display server layer entirely, which ensures compatability with any desktop environment.