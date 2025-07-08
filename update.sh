if [[ $(whoami) != "root" ]]; then
    echo "You have to be acting as root to run this script."
    exit 1
fi
su $SUDO_USER -c 'git pull'
su $SUDO_USER -c 'systemctl --user stop three-finger-drag'
cp ./target/release/linux-3-finger-drag /usr/bin/
su $SUDO_USER -c 'systemctl --user start three-finger-drag'
