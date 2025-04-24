###########################
# linux-three-finger-drag #
#   Installation Script   #
###########################

echo -n "Verifying prerequisites...                      "
if [[ $(whoami) != "root" ]]; then
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\n\e[0;31mFatal\e[0m: Root privileges are needed to install this program"
    echo "and configure the relevant settings (including kernel modules to load at boot)." 
    exit 1
fi

# verify CWD is the repo folder
if [[ ${PWD##*/} != "linux-3-finger-drag" ]]; then
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\n\e[0;31mFatal\e[0m: This script needs to be run from the repo directory"
    echo "(linux-3-finger-drag) to run properly. Either return to that directory,"
    echo "or, if you're already there, change the name back to linux-3-finger-drag."
    exit 1
fi


echo -e "[\e[0;32m DONE \e[0m]"


# 0. Check if libinput tools is installed
# if the command isn't found, stdout will be null
# (since output will be in stderr only, which we won't show)
echo -n "Checking for libinput helper tools...           "
if [[ -z "$(libinput --version 2> /dev/null)" ]]; then
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\n\e[0;31mFatal\e[0m: libinput helper tools are not installed, and are "
    echo "needed to run the program."
    echo "See https://pkgs.org/download/libinput-tools or https://pkgs.org/download/libinput-utils"
    echo -e "for information on installing them for your distro.\n"
    exit 127
else
    echo -e "[\e[0;32m DONE \e[0m]"
fi

# (1. repo already cloned, presumably)

# 2. Disable 3-finger gestures in libinput-gestures
echo -n "Updating libinput-gestures configs...           "
if [[ -d /etc/libinput-gestures.conf ]]; then
    cat /etc/libinput-gestures.conf > /etc/libinput-gestures.conf.bak
    sed -i 's/gesture swipe up/gesture swipe up 4/' /etc/libinput-gestures.conf
    sed -i 's/gesture swipe down/gesture swipe down 4/' /etc/libinput-gestures.conf
    sed -i 's/gesture swipe left/gesture swipe left 4/' /etc/libinput-gestures.conf
    sed -i 's/gesture swipe right/gesture swipe right 4/' /etc/libinput-gestures.conf
    echo "Previous configs saved in /etc/libinput-gestures.conf.bak"
elif [[ -d ~/.config/libinput-gestures.conf ]]; then
    cat ~/.config/libinput-gestures.conf > ~/.config/libinput-gestures.conf.bak
    sed -i 's/gesture swipe up/gesture swipe up 4/' ~/.config/libinput-gestures.conf
    sed -i 's/gesture swipe down/gesture swipe down 4/' ~/.config/libinput-gestures.conf
    sed -i 's/gesture swipe left/gesture swipe left 4/' ~/.config/libinput-gestures.conf
    sed -i 's/gesture swipe right/gesture swipe right 4/' ~/.config/libinput-gestures.conf
    echo "Previous configs saved in ~/.config/libinput-gestures.conf.bak"
fi
echo -e "[\e[0;32m DONE \e[0m]"
echo
echo "The libinput-gestures' config file (if installed) has been updated to "
echo "change 3-finger gestures to 4-finger gestures, to avoid gesture"
echo "ambiguity for the system."
echo
echo "If there are any other services active that use 3-finger gestures,"
echo "please adjust them to use 4 fingers instead (see installation step 2 in the README). "
echo "This avoids ambiguity in your system's input."
echo 
echo -n "Press [Enter] when you have completed this."
read


# 3. Update permissions
echo -n "Updating permissions...                         "
## Update udev rules
mkdir -p /etc/udev/rules.d   # make if not already extant
cp ./60-uinput.rules /etc/udev/rules.d

## Add user to "input" group to read libinput debug events
gpasswd --add $SUDO_USER input

## Automatically load uinput kernel module
## Not necessary on Ubuntu-based distros,
## But essential on Arch (and probably more minimal distros too), 
## and does no harm on other distros
echo "uinput" > /etc/modules-load.d/uinput.conf
modprobe uinput

echo -e "[\e[0;32m DONE \e[0m]"


# 4. Build with Cargo
echo "Compiling..."
echo

## this needs to be done as the user, or else is messes up the permissions
## Cargo should never really be run as root anyway
su $SUDO_USER -c 'cargo build --release'
echo


# 6. Install to /usr/bin
# If you're getting permission issues, try setting the setuid bit (to execute as root) with
#     
#     chmod u+s /usr/bin/linux-3-finger-drag 
#
# This isn't preferred (unless you trust my program), but it will solve the issue.
echo -n "Installing binary to /usr/bin...                "
cp ./target/release/linux-3-finger-drag /usr/bin
echo -e "[\e[0;32m DONE \e[0m]"


# 7. Set up config file
# Has to be done as non-root user, so the file is accessible to the user
echo -n "Installing config file...                       "
su $SUDO_USER -c '\
    mkdir -p ~/.config/linux-3-finger-drag; \
    cp 3fd-config.json ~/.config/linux-3-finger-drag '
echo -e "[\e[0;32m DONE \e[0m]"


# (8a. KDE Autostart needs to be configured through GUI)

# 8b. Installing SystemD service
# If using SystemD as the init system
echo -n "Installing/enabling SystemD user unit...        "
if [[ -n $(ps -p 1 | grep systemd) ]]; then

    # define user-level service
    # made as non-root user
    su $SUDO_USER -c '\
        mkdir -p $HOME/.config/systemd/user; \
        cp three-finger-drag.service $HOME/.config/systemd/user/; \
        systemctl --user enable three-finger-drag.service '
    echo -e "[\e[0;32m DONE \e[0m]"

else
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\n\e[0;33mWarning: Your system doesn't use SystemD.\e[0m"
    echo "Currently, only SystemD installation is automated by this install script,"
    echo "so you'll have to use create and enable the service for your init."
    echo
    echo "You may also have to ensure that the uinput kernel module loads on boot."
    echo "The config was just added in /etc/modules-load.d/uinput.conf."
    echo
    echo "(If I get enough requests for it I'll adapt this install script for other inits,"
    echo "probably starting with OpenRC. Also, feel free to submit a pull request for this.)"
fi

echo
echo "This installation requires a reboot to complete (for the group modification)."
echo
echo -n "Would you like to reboot now? (y/n, default y) "
read answer

case "$answer" in 
    y | "")
        echo "Okay! Rebooting now..."
        #reboot
    ;;
    n)
        echo "Not rebooting. The program will start working after the next boot."
    ;;
    *)
        echo "Response not recognized, not rebooting. The program will start working after the next boot."
esac


echo
echo -e "\e[0;32mInstall complete!\e[0m"
echo