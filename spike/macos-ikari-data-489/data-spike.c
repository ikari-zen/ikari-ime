// スパイク #489 (a)(b)(d): Sandbox 内の ikari-data 相当。捨てるコード。
// 使い方: data-spike <probe|net>   データの場所は pw_dir から決める。
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <pwd.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;

#define STEP(label, expr) do { errno = 0; int r_ = (expr); printf("  %-34s %s (rc=%d errno=%d %s)\n", label, r_ >= 0 ? "OK  " : "FAIL", r_, errno, errno ? strerror(errno) : ""); } while (0)

static char root[1024];
static char p[2048];
static const char *J(const char *a) { snprintf(p, sizeof p, "%s/%s", root, a); return p; }

static int write_file(const char *path) {
  int fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
  if (fd < 0) return -1;
  int n = (int)write(fd, "x", 1);
  close(fd);
  return n;
}

static void net(void) {
  struct addrinfo hints = {0}, *res = NULL;
  hints.ai_socktype = SOCK_STREAM;
  int g = getaddrinfo("huggingface.co", "443", &hints, &res);
  printf("  getaddrinfo rc=%d\n", g);
  if (g) return;
  int s = socket(res->ai_family, SOCK_STREAM, 0);
  errno = 0;
  int c = connect(s, res->ai_addr, res->ai_addrlen);
  printf("  connect:443 rc=%d errno=%d %s\n", c, errno, errno ? strerror(errno) : "");
  close(s);
  // TLS つきの HEAD は子の curl（Sandbox を引き継ぐ）で 1 回だけ
  char *argv[] = {"curl", "-sS", "-I", "-m", "20", "-o", "/dev/null", "-w", "  curl http=%{http_code}\\n", "https://huggingface.co/", NULL};
  pid_t pid;
  int pr = posix_spawn(&pid, "/usr/bin/curl", NULL, NULL, argv, environ);
  printf("  spawn curl rc=%d\n", pr);
  if (!pr) { int st; waitpid(pid, &st, 0); printf("  curl exit=%d\n", WEXITSTATUS(st)); }
}

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "probe";
  struct passwd pw, *res = NULL; char buf[4096];
  getpwuid_r(getuid(), &pw, buf, sizeof buf, &res);
  const char *envhome = getenv("HOME");
  printf("HOME(env)=%s\npw_dir=%s\n", envhome ? envhome : "(null)", res ? pw.pw_dir : "(null)");
  if (!res) return 2;
  snprintf(root, sizeof root, "%s/Library/Application Support/ikari-ime-spike", pw.pw_dir);
  printf("root=%s\n", root);
  if (!strcmp(mode, "net")) { net(); return 0; }

  struct stat st;
  STEP("lstat(root)", lstat(root, &st));
  STEP("mkdir(root) [create_dir_all 第1段]", mkdir(root, 0700));
  puts("-- (a) 例外の中");
  STEP("mkdir(fetch)", mkdir(J("fetch"), 0700));
  errno = 0;
  int lk = open(J("fetch/.lock"), O_RDWR | O_CREAT | O_NOFOLLOW, 0600);
  printf("  %-34s %s (fd=%d errno=%d)\n", "open(fetch/.lock, O_CREAT)", lk >= 0 ? "OK  " : "FAIL", lk, errno);
  if (lk >= 0) STEP("flock(LOCK_EX|LOCK_NB)", flock(lk, LOCK_EX | LOCK_NB));
  STEP("write(fetch/download.bin)", write_file(J("fetch/download.bin")));
  STEP("mkdir(inbox)", mkdir(J("inbox"), 0700));
  STEP("mkdir(inbox/x.partial)", mkdir(J("inbox/x.partial"), 0700));
  STEP("write(inbox/x.partial/f)", write_file(J("inbox/x.partial/f")));
  char from[2048]; snprintf(from, sizeof from, "%s", J("inbox/x.partial"));
  STEP("rename(x.partial -> x)", rename(from, J("inbox/x")));
  struct statvfs sv; STEP("statvfs(fetch)", statvfs(J("fetch"), &sv));
  STEP("lstat(inbox/x/f)", lstat(J("inbox/x/f"), &st));
  puts("-- 例外の外（拒まれるべき）");
  STEP("mkdir(root/other)", mkdir(J("other"), 0700));
  STEP("write(root/other-file)", write_file(J("other-file")));
  STEP("open(root, O_RDONLY)", open(root, O_RDONLY));
  STEP("lstat(root/other)", lstat(J("other"), &st));
  puts("-- HOME 直下");
  char q[2048]; snprintf(q, sizeof q, "%s/spike-outside", pw.pw_dir);
  STEP("mkdir(pw_dir/spike-outside)", mkdir(q, 0700));
  if (lk >= 0) close(lk);
  return 0;
}
