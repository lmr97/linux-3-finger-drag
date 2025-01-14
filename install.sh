###########################
# linux-multi-finger-drag #
#   Installation Script   #
###########################

echo -n "Verifying prerequisites...                                     "
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
echo -n "Checking for libinput helper tools...                          "
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
echo "How many fingers on the trackpad would you like to trigger a drag?"
echo -n "(default 3): "
read fingers

echo -n "Updating config file...                                        "
sed -i "2s/3/$fingers/" mfd-config.json
echo -e "[\e[0;32m DONE \e[0m]"


# 3. Update permissions

## Update udev rules
mkdir -p /etc/udev/rules.d   # make if not already extant
cp ./60-uinput.rules /etc/udev/rules.d

## Automatically load uinput kernel module
## Not necessary on Ubuntu-based distros,
## But essential on Arch (and probably more minimal distros too), 
## and does no harm on other distros
echo -n "Setting uinput modules to load at boot...                      "
echo "uinput" > /etc/modules-load.d/uinput.conf
echo -e "[\e[0;32m DONE \e[0m]"

## Add user to input group, so they can see libinput debug events
echo -n "Adding you to input group...                                   "
gpasswd --add $SUDO_USER input
echo -e "[\e[0;32m DONE \e[0m]"


# 4. Build with Cargo
cargo build --release


# 6. Install to /usr/bin
echo -n "Installing binary to /usr/bin...                               "
cp ./target/release/linux-n-finger-drag /usr/bin
echo -e "[\e[0;32m DONE \e[0m]"


# 7. Set up config file
# Has to be done as non-root user, so the file is accessible to the user
echo -n "Installing config file...                                      "
su $SUDO_USER -c '\
    mkdir -p ~/.config/linux-n-finger-drag; \
    cp 3fd-config.json ~/.config/linux-n-finger-drag '
echo -e "[\e[0;32m DONE \e[0m]"


# (8a. KDE Autostart needs to be configured through GUI)

# 8b. Installing SystemD service
# If using SystemD as the init system
echo -n "Installing/enabling SystemD service...                         "
if [[ -n $(ps -p 1 | grep systemd) ]]; then

    # define user-level service
    su $SUDO_USER -c '\
        mkdir -p $HOME/.config/systemd/user; \
        cp multi-finger-drag.service $HOME/.config/systemd/user/; \
        systemctl --user enable multi-finger-drag.service '
    echo -e "[\e[0;32m DONE \e[0m]"

else
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\e[0;33mWarning: It looks like your system doesn't use systemd.\e[0m"
    echo "Currently, only systemd installation is automated by this install script,"
    echo "so you'll have to use create and enable the service for your init service."
    echo "If I get enough requests for it I'll adapt this install script for other inits,"
    echo "probably starting with OpenRC."
    echo -e "[\e[0;31m FAIL \e[0m]"
fi

echo 
echo "This installation requires a reboot to update permissions before running."
echo "Three-finger dragging is set to be active afterward."
echo
echo -n "Would you like to reboot now? [y/n, default y] "
read answer2

if   [ $answer2 = "n" ] || [ $answer2 = "N" ]; then
    exit 0
elif [ $answer2 = "y" ] || [ $answer2 = "Y" ] || [ -z $answer2 ]; then
    reboot
else
    echo "Unrecognized response detected. Exiting without rebooting."
    exit 0
fi

