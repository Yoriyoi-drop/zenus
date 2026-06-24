#![no_std]

use zutils_common::Writer;

pub fn execute<W: Writer + ?Sized>(w: &mut W) {
    w.write_str("Commands:\r\n");
    w.write_str("  help         Show this help\r\n");
    w.write_str("  echo         Print text\r\n");
    w.write_str("  ls           List directory\r\n");
    w.write_str("  ls -l        List with permissions\r\n");
    w.write_str("  cat          Show file contents\r\n");
    w.write_str("  clear        Clear screen\r\n");
    w.write_str("  timer        Show APIC timer tick count\r\n");
    w.write_str("  ps           List processes\r\n");
    w.write_str("  kill         Kill process\r\n");
    w.write_str("  mkdir        Create directory\r\n");
    w.write_str("  rm           Remove file/directory\r\n");
    w.write_str("  touch        Create empty file\r\n");
    w.write_str("  chmod        Change file permissions\r\n");
    w.write_str("  ifconfig     Show network interfaces\r\n");
    w.write_str("  meminfo      Show memory usage\r\n");
    w.write_str("  reboot       Reboot the system\r\n");
    w.write_str("  shutdown     Shutdown the system\r\n");
    w.write_str("  uname        Show kernel version info\r\n");
    w.write_str("  version      Alias for uname\r\n");
    w.write_str("  dmesg        Show kernel log buffer\r\n");
    w.write_str("  id           Show current user/group IDs\r\n");
    w.write_str("  whoami       Show current username\r\n");
    w.write_str("  mount        Show mount points\r\n");
    w.write_str("  pwd          Print working directory\r\n");
    w.write_str("  grep         Search file contents\r\n");
    w.write_str("  find         Find files\r\n");
    w.write_str("  df           Show disk free space\r\n");
    w.write_str("  du           Show disk usage\r\n");
    w.write_str("  cp           Copy file\r\n");
    w.write_str("  mv           Move file\r\n");
    w.write_str("  chown        Change file owner\r\n");
}
