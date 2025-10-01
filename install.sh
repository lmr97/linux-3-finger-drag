###########################
# linux-three-finger-drag #
#   Installation Script   #
###########################

# this flag pops up in several places, so if I change it, 
# I want the changes to be consistent
LIBINPUT_INSTALLED_FLAG="--libinput-installed"

echo -ne "Verifying prerequisites...                      "
if [[ $(whoami) != "root" ]]; then
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\n\e[0;31mFatal\e[0m: Root privileges are needed to install this program \
        and configure the relevant settings (including kernel modules to load at boot)." 
    exit 1
fi


ensure-libinput() {
    # determine package manager
    SEARCH_CMD="uncommon"  # default

    declare -A searchCommand;
    searchCommand["debian"]="apt list -qq --installed libinput-dev"
    searchCommand["ubuntu"]="apt list -qq --installed libinput-dev"         # just in case
    searchCommand["ubuntu debian"]="apt list -qq --installed libinput-dev"  # for PopOS
    searchCommand["redhat"]="dnf list --installed libinput-devel"
    searchCommand["fedora"]="dnf list --installed libinput-devel"
    searchCommand["arch"]="pacman -Qs libinput"
    searchCommand["suse"]="zypper search --installed-only libinput-devel"

    # set all variables from the os-release file
    source /etc/os-release

    if   [[ -n $ID_LIKE ]]; then
        SEARCH_CMD=${searchCommand[${ID_LIKE}]}
    elif [[ -n $ID ]]; then
        SEARCH_CMD=${searchCommand[${ID}]}
    fi

    if [[ $SEARCH_CMD = "uncommon" || -z $SEARCH_CMD ]]; then
        echo -e "[\e[0;31m FAIL \e[0m]"
        echo -e "\nIt looks like you're on an uncommon distribution, which the automatic \
            installer in this script doesn't support (yet). So go ahead and install the \
            libinput development library (it should be named something like 'libinput-dev' \
            in your distribution's package repo), and when that's done, come back and \
            re-run this script with the flag $LIBINPUT_INSTALLED_FLAG"
        exit 127

    elif [[ -z $($SEARCH_CMD 2> /dev/null) ]]; then
        echo -en "\r\e[0;33mlibinput dev library not found, installing...   \e[0m"
        
        INSTALL_CMD="uncommon"  # default

        declare -A installCommand;
        installCommand["debian"]="apt-get -q install -y libinput-dev"
        installCommand["ubuntu"]="apt-get -q install -y libinput-dev"          # just in case
        installCommand["ubuntu debian"]="apt-get -q install -y libinput-dev"   # for PopOS
        installCommand["redhat"]="dnf -y install libinput-devel"
        installCommand["fedora"]="dnf -y install libinput-devel"
        installCommand["arch"]="pacman -S --noconfirm libinput"
        installCommand["suse"]="zypper install -y libinput-devel"


        if   [[ -n $ID_LIKE ]]; then
            INSTALL_CMD=${installCommand[${ID_LIKE}]}
        elif [[ -n $ID ]]; then
            INSTALL_CMD=${installCommand[${ID}]}
        fi

        if [[ $INSTALL_CMD = "uncommon" || -z $SEARCH_CMD ]]; then
            echo -e "[\e[0;31m FAIL \e[0m]"
            echo -e "\nIt looks like you're on an uncommon distribution, which the automatic \
                installer in this script doesn't support (yet). So go ahead and install the \
                libinput development library (it should be named something like 'libinput-dev' \ 
                in your distribution's package repo), and when that's done, come back and \
                re-run this script with the flag $LIBINPUT_INSTALLED_FLAG"
            exit 127
        # probably redundant, but I want to make sure this case is caught
        else
            $INSTALL_CMD > /dev/null 2>&1
            if [[ $? -ne 0 ]]; then
                echo -e "[\e[0;31m FAIL \e[0m]"
                echo -e "\nIt looks like there was an issue with installing the libinput development \
                    library, which were not available on the system when this script started. \
                    They need to be installed prior to the installation of this program, so go \
                    ahead and install the libinput development library (it should be named \
                    something like 'libinput-dev' in your distribution's package repo), and \
                    when that's done, come back and re-run this script with the flag \ 
                    $LIBINPUT_INSTALLED_FLAG"
                exit 127
            fi
        fi
    fi
}

# 1. Check if libinput dev library is installed
if [[ $1 != "$LIBINPUT_INSTALLED_FLAG" ]]; then
    ensure-libinput
else
    echo -en "\r\e[0;33mSkipping check for libinput dev library...      \e[0m"
fi

# verify CWD is the repo folder
if [[ ${PWD##*/} != "linux-3-finger-drag" ]]; then
    echo -e "[\e[0;31m FAIL \e[0m]"
    echo -e "\n\e[0;31mFatal\e[0m: This script needs to be run from the repo directory \
        (linux-3-finger-drag) to run properly. Either return to that directory, \
        or, if you're already there, change the name back to linux-3-finger-drag."
    exit 1
fi

echo -e "[\e[0;32m DONE \e[0m]"

# (2. repo already cloned, presumably)


# 3. Update permissions
echo -n "Updating permissions...                         "
## Update udev rules
mkdir -p /etc/udev/rules.d   # make if not already extant
cp ./60-uinput.rules /etc/udev/rules.d/

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

## this needs to be done as the user, or else is messes up the permissions.
## Cargo should never really be run as root anyway.
su $SUDO_USER -c 'cargo build --release'
CARGO_EXIT_CODE=$?

if [ $CARGO_EXIT_CODE -ne 0 ]; then
    echo -e "\n\e[0;33mHint:\e[0m You probably need to install the libinput development library, \
        which package is typically named something like 'libinput-dev' for your distribution. \
        Some distributions bundle it with their libinput package, too. Once you've installed \
        the package, you can re-run this script with the $LIBINPUT_INSTALLED_FLAG flag (it \
        won't hurt anything)." 
    exit $CARGO_EXIT_CODE
fi
echo


# 5. Install to /usr/bin
# If you're getting permission issues (after a reboot), try setting the 
# setuid bit, so it will execute as root, by running
#     
#     chmod u+s /usr/bin/linux-3-finger-drag 
#
# You can also change its owner to root by a simple:
#
#     chown root:root /usr/bin/linux-3-finger-drag
# 
# Neither of these are preferred security-wise (unless you trust my program), 
# but it should solve the issue.
echo -n "Installing binary to /usr/bin...                "

# with the user added to the 'input' group, root does not need to
# own the executable, and stick to the principle of least privilege.
cp --preserve=ownership ./target/release/linux-3-finger-drag /usr/bin/
echo -e "[\e[0;32m DONE \e[0m]"


# Set up config file
# Has to be done as non-root user, so the file is accessible to the user
echo -n "Installing config file...                       "
su $SUDO_USER -c '\
    mkdir -p ~/.config/linux-3-finger-drag; \
    cp 3fd-config.json ~/.config/linux-3-finger-drag '
echo -e "[\e[0;32m DONE \e[0m]"


# (7a. KDE Autostart needs to be configured through GUI)

# 7b. Installing SystemD service
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
    echo -e "[\e[0;33m WARN \e[0m]"
    echo -e "\n\e[0;33mWarning: Your system doesn't use SystemD.\e[0m"
    echo "Currently, only SystemD installation is automated by this install script, \
        so you'll have to use create and enable the service for your init system."
    echo
    echo "You may also have to ensure that the uinput kernel module loads on boot."
    echo "The config has been added in /etc/modules-load.d/uinput.conf."
    echo
    echo "(If I get enough requests for it I'll adapt this install script for other inits, \
        probably starting with OpenRC. Also, feel free to submit a pull request for this.)"
fi

## 6. Reboot
echo
echo "This installation requires a reboot to complete (for the group modification)."
echo
echo -n "Would you like to reboot now? (y/n, default y) "
read answer

case "$answer" in 
    y | "")
        echo "Okay! Rebooting now..."
        reboot
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
