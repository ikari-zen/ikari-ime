// スパイク #536（捨てるコード。マージしない）。本物の別プロセスの檻役の役（../cage-peer-check-spike/cage.c）を
// SIGSTOP / SIGKILL したとき、殻（名前の口。待ち 3 秒）とエンジン（匿名の口の hello。待ち 5 秒）の待ちが
// どれだけになるかを測る。待ちの長さは製品の値（LMCageBroker.fetchTimeout、HELLO_REPLY_DEADLINE）に合わせる。
//   host <回数>
#include <dispatch/dispatch.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <xpc/xpc.h>

#define SERVICE "io.github.ikari-zen.spike.cage"
#define FETCH_TIMEOUT_NS (3LL * NSEC_PER_SEC)
#define HELLO_TIMEOUT_NS (5LL * NSEC_PER_SEC)

static double now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static const char *describe(xpc_object_t obj) {
    if (!obj) return "timeout";
    if (obj == XPC_ERROR_CONNECTION_INVALID) return "invalid";
    if (obj == XPC_ERROR_CONNECTION_INTERRUPTED) return "interrupted";
    if (xpc_get_type(obj) == XPC_TYPE_ERROR) return "error";
    if (xpc_get_type(obj) == XPC_TYPE_DICTIONARY) return "dictionary";
    return "other";
}

// 送って、返事を timeout まで待つ（製品と同じ: 非同期の返事 + セマフォ）
static xpc_object_t ask(xpc_connection_t c, const char *op, int64_t timeout_ns) {
    __block xpc_object_t got = NULL;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    xpc_object_t m = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(m, "op", op);
    xpc_dictionary_set_uint64(m, "v", 1);
    xpc_connection_send_message_with_reply(c, m, dispatch_get_global_queue(0, 0), ^(xpc_object_t r) {
        got = xpc_retain(r);
        dispatch_semaphore_signal(sem);
    });
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, timeout_ns)) != 0) return NULL;
    return got;
}

static xpc_connection_t open_named(void) {
    xpc_connection_t c = xpc_connection_create(SERVICE, NULL);
    xpc_connection_set_event_handler(c, ^(xpc_object_t e) { (void)e; });
    xpc_connection_resume(c);
    return c;
}

static xpc_connection_t open_anon(xpc_endpoint_t ep) {
    xpc_connection_t c = xpc_connection_create_from_endpoint(ep);
    xpc_connection_set_event_handler(c, ^(xpc_object_t e) { (void)e; });
    xpc_connection_resume(c);
    return c;
}

int main(int argc, char **argv) {
    int n = argc > 1 ? atoi(argv[1]) : 5;
    for (int i = 1; i <= n; i++) {
        // 0) ふつうに口を取る（檻役は動いている）
        xpc_connection_t named = open_named();
        double t = now_ms();
        xpc_object_t r = ask(named, "endpoint", FETCH_TIMEOUT_NS);
        double fetch_ok = now_ms() - t;
        if (!r || xpc_get_type(r) != XPC_TYPE_DICTIONARY) {
            printf("SPIKE536-PROC i=%d fetch failed: %s\n", i, describe(r));
            return 1;
        }
        xpc_endpoint_t ep = (xpc_endpoint_t)xpc_retain(xpc_dictionary_get_value(r, "endpoint"));
        pid_t pid = xpc_connection_get_pid(named);
        // 1) 止めた状態（SIGSTOP）: エンジンの hello と、殻の名前の口の取り直し
        kill(pid, SIGSTOP);
        xpc_connection_t anon = open_anon(ep);
        t = now_ms();
        r = ask(anon, "hello", HELLO_TIMEOUT_NS);
        double hello_stopped = now_ms() - t;
        const char *hello_stopped_what = describe(r);
        xpc_connection_t named2 = open_named();
        t = now_ms();
        r = ask(named2, "endpoint", FETCH_TIMEOUT_NS);
        double fetch_stopped = now_ms() - t;
        const char *fetch_stopped_what = describe(r);
        kill(pid, SIGCONT);
        usleep(200 * 1000);
        // 2) 落ちた状態（SIGKILL）: 古い口への hello と、名前の口の取り直し（launchd が起こし直す）
        kill(pid, SIGKILL);
        usleep(200 * 1000);
        xpc_connection_t anon2 = open_anon(ep);
        t = now_ms();
        r = ask(anon2, "hello", HELLO_TIMEOUT_NS);
        double hello_dead = now_ms() - t;
        const char *hello_dead_what = describe(r);
        xpc_connection_t named3 = open_named();
        t = now_ms();
        r = ask(named3, "endpoint", FETCH_TIMEOUT_NS);
        double fetch_relaunch = now_ms() - t;
        const char *fetch_relaunch_what = describe(r);
        pid_t p3 = xpc_connection_get_pid(named3);
        printf("SPIKE536-PROC i=%d pid=%d fetch_ok=%.1fms | stopped: hello=%.1fms(%s) fetch=%.1fms(%s) | "
               "killed: hello_old_ep=%.1fms(%s) fetch_relaunch=%.1fms(%s) newpid=%d\n",
               i, pid, fetch_ok, hello_stopped, hello_stopped_what, fetch_stopped, fetch_stopped_what, hello_dead,
               hello_dead_what, fetch_relaunch, fetch_relaunch_what, p3);
        fflush(stdout);
        xpc_connection_cancel(anon);
        xpc_connection_cancel(anon2);
        xpc_connection_cancel(named);
        xpc_connection_cancel(named2);
        xpc_connection_cancel(named3);
        // 次の回のために、起こし直した檻役の役（このスパイクのプロセスだけ）も止める
        // launchd は続けて落ちたサービスの起こし直しを遅らせるので、回の間を空ける（argv[2] 秒）
        if (p3 > 0) kill(p3, SIGKILL);
        sleep(argc > 2 ? (unsigned)atoi(argv[2]) : 15);
    }
    return 0;
}
