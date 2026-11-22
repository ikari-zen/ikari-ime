// スパイク #578（ADR 0080 C1）。捨てるコード。マージしない。
// XPC サービス（launchd が起こす。App Sandbox）。要求が来たら、自分の束の Resources/data/ の 4 ファイルを
// mmap し、mincore で常駐の割合を見てから全ページに触り、結果を文字列で返す。"deny" のパスは読めないはず（対照）。
#include <xpc/xpc.h>
#include <mach-o/dyld.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <libgen.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

extern int sandbox_check(pid_t pid, const char *operation, int type, ...);

static const char *FILES[] = {
  "dict/system.dict", "dict/grammar.bin", "dict/vectors.bin", "models/zenz-v3.2-small-Q5_K_M.gguf",
};
enum { NFILES = sizeof FILES / sizeof FILES[0], OUT_MAX = 8192 };

static double now_ms(void) { return clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1e6; }

static double resident(void *p, size_t len, size_t ps) {
  size_t pages = (len + ps - 1) / ps;
  char *v = malloc(pages);
  if (!v || mincore(p, len, v) != 0) { free(v); return -1; }
  size_t n = 0;
  for (size_t i = 0; i < pages; i++) n += v[i] & 1;
  free(v);
  return n * 100.0 / pages;
}

static void probe(const char *deny, char *out, size_t cap) {
  size_t used = 0;
#define OUT(...) used += snprintf(out + used, used < cap ? cap - used : 0, __VA_ARGS__)
  char exe[PATH_MAX]; uint32_t sz = sizeof exe;
  _NSGetExecutablePath(exe, &sz);
  char root[PATH_MAX];
  snprintf(root, sizeof root, "%s/../Resources/data", dirname(exe));   // <束>/Contents/MacOS/..
  OUT("pid=%d sandboxed=%d HOME=%s\nroot=%s\n", getpid(), sandbox_check(getpid(), NULL, 0), getenv("HOME"), root);
  size_t ps = (size_t)getpagesize();
  double all = now_ms(); size_t total = 0;
  for (int i = 0; i < NFILES; i++) {
    char path[PATH_MAX]; snprintf(path, sizeof path, "%s/%s", root, FILES[i]);
    double t0 = now_ms();
    int fd = open(path, O_RDONLY);
    if (fd < 0) { OUT("%s: open FAILED errno=%d (%s)\n", FILES[i], errno, strerror(errno)); continue; }
    struct stat st; fstat(fd, &st);
    void *p = mmap(NULL, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (p == MAP_FAILED) { OUT("%s: mmap FAILED errno=%d\n", FILES[i], errno); close(fd); continue; }
    double t_map = now_ms() - t0;
    double before = resident(p, st.st_size, ps);
    double t1 = now_ms(); volatile unsigned long sum = 0;
    for (off_t o = 0; o < st.st_size; o += ps) sum += ((volatile unsigned char *)p)[o];
    double t_touch = now_ms() - t1;
    double after = resident(p, st.st_size, ps);
    OUT("%s: size=%.1fMB mmap=%.2fms resident_before=%.1f%% touch_all=%.1fms resident_after=%.1f%%\n",
        FILES[i], st.st_size / 1e6, t_map, before, t_touch, after);
    total += st.st_size;
    munmap(p, st.st_size); close(fd);
  }
  OUT("mmap total: %.1fMB in %.1fms\n", total / 1e6, now_ms() - all);
  if (deny && *deny) {
    int fd = open(deny, O_RDONLY);
    if (fd >= 0) { OUT("deny-check: OPEN OK (Sandbox が効いていない)\n"); close(fd); }
    else OUT("deny-check: %s: errno=%d (%s)\n", deny, errno, strerror(errno));
  }
#undef OUT
}

static void on_peer(xpc_connection_t peer) {
  xpc_connection_set_event_handler(peer, ^(xpc_object_t ev) {
    if (xpc_get_type(ev) != XPC_TYPE_DICTIONARY) return;
    char out[OUT_MAX];
    probe(xpc_dictionary_get_string(ev, "deny"), out, sizeof out);
    xpc_object_t r = xpc_dictionary_create_reply(ev);
    if (!r) return;
    xpc_dictionary_set_string(r, "out", out);
    xpc_connection_send_message(peer, r);
    xpc_release(r);
  });
  xpc_connection_resume(peer);
}

int main(void) { xpc_main(on_peer); }
