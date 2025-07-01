# Three Finger Drag for Wayland/KDE
This program provides three-finger-drag support computers with touchpads running in Wayland sessions (notably KDE Plasma 6). It only depends on `libinbut` and `uinput`, so the program should run in any desktop environment that has `libinput` installed, regardless of whether it uses X11 or Wayland.

## Tested on...

OS | Version | Desktop Enviroment | Verified
---|---|---|---
**Kubuntu** | 24.10 | KDE Plasma 6 | ✅ <sup>1</sup>
**Pop_OS!** | (unknown) | COSMIC | ✅
**Arch** | 2025-01-01 | GNOME | ✅
**Gentoo** | 6.6.67 | KDE Plasma 6 | ✅
**Fedora** | 42 | GNOME (v. 48) | ✅

---
<sup>1</sup> Developed on this setup.

## What is three-finger dragging?

Three-finger dragging is a feature originally for trackpads on Mac devices: instead of holding down the left click on the pad to drag, you can simply rest three fingers on the trackpad to start a mouse hold, and move the fingers together to continue the drag in whatever direction you move them in. In short, it interprets three fingers on the trackpad as a mouse-down input, and motion with three fingers afterwards for mouse movement. It can be quite handy, as it will save your hand some effort for moving windows around and highlighting text. 

Here is [an example](https://www.youtube.com/watch?v=-Fy6imaiHWE) of three-finger dragging in action on a MacBook.

## Automated installation

The included `install.sh` installs the program as a SystemD user unit (other inits are not yet supported). It also updates the `libinput-gestures` config files (if you have that installed) so that all 3-finger gestures become 4-finger gestures. 

It requires the following to run properly: 
* **Root permissions**
* A working Rust installation (see [Rust's install guide](https://www.rust-lang.org/tools/install))
* `libinput`'s helper tools (see Step 0 below for install details)

It will also ask to reboot your system afterward, which is required to update permissions for `libinput` and `uinput`.

You can execute the install script with the following:

```
sudo bash install.sh
```

## Manual installation

### 0. Install `libinput` (you probably have it already)

If you are using GNOME, KDE Plasma, or an Xorg-based desktop environment, you already have `libinput`'s dev package installed (it's a dependency of those environments). To make sure, you check with your package manager to check whether you have `libinput-dev` or `libinput-devel` installed (depending on the distro). It may also simply be included with the `libinput` package.


### 1. Clone the repository
```
git clone https://github.com/lmr97/linux-3-finger-drag.git
cd linux-3-finger-drag
```

### 2. Change any existing 3-finger gestures to 4-finger gestures

If you already like your existing 3-finger gestures, but still want a multi-finger drag, check out the [`n-finger-drag` branch](https://github.com/lmr97/linux-3-finger-drag/tree/n-finger-drag) of this repostory, where the number of fingers to trigger the drag is configurable. It's still experimental, so any feedback would be welcome!

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

This programs reads from `libinput` (using `/dev/input/event0`), and writes to `/dev/uinput`, so it requires an adjustment of permissions to accomplish both. 

#### 3.1 For `uinput`
For more info about what's being done here, see [this section](https://wiki.archlinux.org/title/Udev#Allowing_regular_users_to_use_devices) of the ArchWiki article on `udev`. 
You may need to create the folder `rules.d` in `/etc/udev`.

<u>**For Arch users**</u>: You will need to set the `uinput` kernel module to load on boot, if you haven't already, line so:
```
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf 2&>/dev/null
```

Add the udev rules to your system:
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

### 5. Verify funtionality
Check to make sure the executable works by running
```
./target/release/linux-3-finger-drag
```

You will see a warning about not being able to find a configuration file. Just ignore that for now, as it will not impact functionality; we'll get to configuration in step 7. 

I've tried to make error messages as informative as I can, with recommended actions included to remedy them, so look to those error messages for help first. But if they don't help, please submit an issue here, with information about your OS, desktop environment, and any error output you get, and I'll get on it as soon as I can. 

### 6. Install into `/usr/bin`
Once you've got it working, copy it into `/usr/bin` for ease and consistency of access:
```
sudo cp ./target/release/linux-3-finger-drag /usr/bin
```

### 7. Set up configuration file (optional)

See the Configuration section below about the included `3fd-config.json` file. If you'd like to configure the behavior of this program, run the following:
```
mkdir ~/.config/linux-3-finger-drag
cp 3fd-config.json ~/.config/linux-3-finger-drag
```
Now you can configure to your heart's content!

### 8. Add program to Autostart (KDE only)
This is a part of the graphical interface. You can find the Autostart menu in System Settings > Autostart (near the bottom). Once there, click the "+ Add..." button in the upper right of the window, and select "Add Application" from the dropdown menu. Then, in text bar in the window that pops up, paste
```
/usr/bin/linux-3-finger-drag
```
and click OK. 

Now select the program in the Autostart menu, and press Start in the upper right-hand corner of the window to start using it in the current session. It will automatically start in the next session you log into.

### 8b. Add program to SystemD (works distro and desktop agnostic)

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

### 9. Reboot

A reboot is required to update all the permissions needed for the program to run.

### You did it! Now you can 3-finger-drag!


## Configuration
There is a JSON configuration file, assumed to be in `~/.config/linux-3-finger-drag/` called `3fd-config.json`, which is read into the program at startup. You can specify an acceleration value (`acceleration`), which will be multiplied with all 3-finger gesture movements. You can also specify the time (in milliseconds) that the mouse hold will persist for after you lift your fingers (to give you a moment to reposition your fingers), with `drag_end_delay`. It's entirely optional: if the file cannot be read for any reason, the program will simply warn the user that the file could not be read (with the reason), and default to an acceleration multiplier of 1 and a drag end delay value of 0. 

## How it works
This program uses Rust bindings for libinput to detect three-finger gestures, and translates them into write-calls to [`/dev/uinput`](https://www.kernel.org/doc/html/v4.12/input/uinput.html) via a virtual trackpad. This flow of control bypasses the display server layer entirely, which ensures compatability with any desktop environment (at least with some modifications).