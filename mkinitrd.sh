#!/bin/bash
set -e
ORIG_DIR=$(pwd)
INITRD_DIR=$(mktemp -d)
mkdir -p "$INITRD_DIR/init" "$INITRD_DIR/bin" "$INITRD_DIR/etc"
echo "Hello from bin!" > "$INITRD_DIR/bin/hello"
echo "Zenus OS v0.1.0 - Server OS" > "$INITRD_DIR/version.txt"
echo "Hello from initrd!" > "$INITRD_DIR/hello.txt"
echo "Zenus OS - Server Mode" > "$INITRD_DIR/etc/motd"
cat > "$INITRD_DIR/init/startup.sh" << 'SCRIPT'
#!/bin/sh
echo "Zenus initrd startup"
cat /initrd/etc/motd
SCRIPT
chmod +x "$INITRD_DIR/init/startup.sh"
cd "$INITRD_DIR" && tar cf "$ORIG_DIR/$1" .
rm -rf "$INITRD_DIR"
echo "Created initrd: $1"
