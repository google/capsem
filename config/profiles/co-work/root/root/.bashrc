# Capsem profile shell bootstrap.
export PATH="/root/.local/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

if [ -f /root/tips.txt ]; then
    sed -n '1,3p' /root/tips.txt
fi
