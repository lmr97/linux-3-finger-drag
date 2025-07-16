# This is an update script intended to be run as root, to update
# the program for those who run it with systemd.
if [[ $(whoami) != "root" ]]
then
    echo "You have to be acting as root to run this script."
    exit 1
fi

if [[ ${PWD##*/} != "linux-3-finger-drag" ]]
then
    echo "This script needs to be run from the repository directory."
    exit 1
fi

su $SUDO_USER -c 'git pull'
su $SUDO_USER -c 'systemctl --user stop three-finger-drag'
su $SUDO_USER -c 'cargo build --release'
cp --preserve=ownership ./target/release/linux-3-finger-drag /usr/bin/
su $SUDO_USER -c 'systemctl --user start three-finger-drag'
