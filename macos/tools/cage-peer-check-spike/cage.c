// スパイク（捨てるコード。製品では使わない）。ADR 0077 §10 の最後の推測、#448 の FC-1 の確かめ。
// 檻役の役の XPC サービス。殻の役（host.c）に匿名の口を渡し、その口に届いた伝言をログに出して返事をする。
#include <os/log.h>
#include <stdio.h>
#include <unistd.h>
#include <xpc/xpc.h>

static os_log_t logger;
// 匿名の口の受け手（プロセスの寿命の間持つ）
static xpc_connection_t listener;

static void on_anonymous_peer(xpc_connection_t peer) {
    xpc_connection_set_event_handler(peer, ^(xpc_object_t msg) {
        if (xpc_get_type(msg) != XPC_TYPE_DICTIONARY) {
            os_log(logger, "cage: anonymous peer event: %{public}s",
                   xpc_get_type(msg) == XPC_TYPE_ERROR ? "error" : "other");
            return;
        }
        const char *op = xpc_dictionary_get_string(msg, "op");
        int fd = xpc_dictionary_dup_fd(msg, "model");
        os_log(logger, "cage: received op=%{public}s fd=%{public}s pid=%d",
               op ? op : "(none)", fd >= 0 ? "present" : "absent",
               xpc_connection_get_pid(peer));
        if (fd >= 0) {
            close(fd);
        }
        xpc_object_t reply = xpc_dictionary_create_reply(msg);
        if (reply) {
            xpc_dictionary_set_string(reply, "reply", "ok");
            xpc_dictionary_set_uint64(reply, "v", 1);
            xpc_connection_send_message(peer, reply);
        }
    });
    xpc_connection_resume(peer);
}

static void on_named_peer(xpc_connection_t peer) {
    xpc_connection_set_event_handler(peer, ^(xpc_object_t msg) {
        if (xpc_get_type(msg) != XPC_TYPE_DICTIONARY) {
            return;
        }
        const char *op = xpc_dictionary_get_string(msg, "op");
        os_log(logger, "cage: named op=%{public}s", op ? op : "(none)");
        xpc_object_t reply = xpc_dictionary_create_reply(msg);
        if (!reply) {
            return;
        }
        if (!listener) {
            listener = xpc_connection_create(NULL, NULL);
            xpc_connection_set_event_handler(listener, ^(xpc_object_t event) {
                if (xpc_get_type(event) == XPC_TYPE_CONNECTION) {
                    on_anonymous_peer((xpc_connection_t)event);
                } else {
                    os_log(logger, "cage: listener event (not a connection)");
                }
            });
            xpc_connection_resume(listener);
        }
        xpc_object_t endpoint = xpc_endpoint_create(listener);
        xpc_dictionary_set_value(reply, "endpoint", endpoint);
        xpc_connection_send_message(peer, reply);
    });
    xpc_connection_resume(peer);
}

int main(void) {
    logger = os_log_create("io.github.ikari-zen.spike", "cage");
    os_log(logger, "cage: start");
    xpc_main(on_named_peer);
    return 0;
}
