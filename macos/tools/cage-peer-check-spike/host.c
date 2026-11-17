// スパイク（捨てるコード。製品では使わない）。ADR 0077 §10 の最後の推測、#448 の FC-1 の確かめ。
// 殻とエンジンの役を 1 つにしたもの（Sandbox なし）。檻役の役（cage.c）から匿名の口をもらい、つなぐ側から
// 相手の署名要件を確かめられるかを試す。
//
//   host match     つなぐ側に、檻役に合う署名要件を付けて hello を送る
//   host mismatch  合わない署名要件を付けて hello を送る（伝言が相手に届くか、返事がどうなるかを見る）
//   host seccode   署名要件を付けずに送り、返事の伝言から相手のコードを取り、自分で要件を確かめる
//
// 確かめのために、hello にファイルの fd を付ける（製品の hello は fd を付けない）。
// 環境変数 SPIKE_TEAM があれば、要件にチーム ID を足す（team の署名で試すとき）。
#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <xpc/xpc.h>

#define SERVICE "io.github.ikari-zen.spike.cage"
#define OTHER_ID "io.github.ikari-zen.spike.other"

static void requirement(char *out, size_t size, const char *identifier) {
    const char *team = getenv("SPIKE_TEAM");
    if (team && *team) {
        snprintf(out, size,
                 "anchor apple generic and identifier \"%s\" and certificate leaf[subject.OU] = \"%s\"",
                 identifier, team);
    } else {
        snprintf(out, size, "identifier \"%s\"", identifier);
    }
}

static const char *describe(xpc_object_t obj) {
    if (obj == XPC_ERROR_CONNECTION_INVALID) return "error:connection-invalid";
    if (obj == XPC_ERROR_CONNECTION_INTERRUPTED) return "error:connection-interrupted";
    if (obj == XPC_ERROR_PEER_CODE_SIGNING_REQUIREMENT) return "error:peer-code-signing-requirement";
    if (xpc_get_type(obj) == XPC_TYPE_ERROR) return "error:other";
    if (xpc_get_type(obj) == XPC_TYPE_DICTIONARY) return "dictionary";
    return "other";
}

static xpc_endpoint_t fetch_endpoint(void) {
    xpc_connection_t named = xpc_connection_create(SERVICE, NULL);
    xpc_connection_set_event_handler(named, ^(xpc_object_t event) {
        printf("host: named event %s\n", describe(event));
    });
    xpc_connection_resume(named);
    xpc_object_t msg = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(msg, "op", "endpoint");
    xpc_object_t reply = xpc_connection_send_message_with_reply_sync(named, msg);
    printf("host: endpoint reply %s\n", describe(reply));
    if (xpc_get_type(reply) != XPC_TYPE_DICTIONARY) {
        return NULL;
    }
    xpc_object_t ep = xpc_dictionary_get_value(reply, "endpoint");
    if (!ep || xpc_get_type(ep) != XPC_TYPE_ENDPOINT) {
        printf("host: no endpoint in reply\n");
        return NULL;
    }
    return (xpc_endpoint_t)xpc_retain(ep);
}

static void check_reply_code(xpc_object_t reply) {
    SecCodeRef code = NULL;
    OSStatus st = SecCodeCreateWithXPCMessage(reply, kSecCSDefaultFlags, &code);
    printf("host: SecCodeCreateWithXPCMessage status=%d\n", (int)st);
    if (st != errSecSuccess) {
        return;
    }
    const char *ids[2] = {SERVICE, OTHER_ID};
    for (int i = 0; i < 2; i++) {
        char text[512];
        requirement(text, sizeof text, ids[i]);
        CFStringRef s = CFStringCreateWithCString(NULL, text, kCFStringEncodingUTF8);
        SecRequirementRef req = NULL;
        OSStatus rs = SecRequirementCreateWithString(s, kSecCSDefaultFlags, &req);
        OSStatus vs = rs == errSecSuccess ? SecCodeCheckValidity(code, kSecCSDefaultFlags, req) : rs;
        printf("host: seccode requirement=[%s] status=%d\n", text, (int)vs);
        if (req) CFRelease(req);
        CFRelease(s);
    }
    CFRelease(code);
}

int main(int argc, char **argv) {
    const char *mode = argc > 1 ? argv[1] : "";
    if (strcmp(mode, "match") && strcmp(mode, "mismatch") && strcmp(mode, "seccode")) {
        fprintf(stderr, "usage: host match|mismatch|seccode\n");
        return 2;
    }
    xpc_endpoint_t ep = fetch_endpoint();
    if (!ep) {
        return 1;
    }
    xpc_connection_t conn = xpc_connection_create_from_endpoint(ep);
    xpc_connection_set_event_handler(conn, ^(xpc_object_t event) {
        printf("host: anonymous event %s\n", describe(event));
    });
    if (strcmp(mode, "seccode") != 0) {
        char text[512];
        requirement(text, sizeof text, strcmp(mode, "match") == 0 ? SERVICE : OTHER_ID);
        int rc = xpc_connection_set_peer_code_signing_requirement(conn, text);
        printf("host: set_peer_code_signing_requirement [%s] rc=%d\n", text, rc);
    }
    xpc_connection_resume(conn);

    char path[] = "/tmp/ikari-cage-spike-model.XXXXXX";
    int fd = mkstemp(path);
    if (fd >= 0) {
        close(fd);
        fd = open(path, O_RDONLY);
        unlink(path);
    }
    xpc_object_t hello = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(hello, "op", "hello");
    xpc_dictionary_set_uint64(hello, "v", 1);
    if (fd >= 0) {
        xpc_dictionary_set_fd(hello, "model", fd);
        close(fd);
    }
    xpc_object_t reply = xpc_connection_send_message_with_reply_sync(conn, hello);
    printf("host: mode=%s hello reply %s\n", mode, describe(reply));
    if (xpc_get_type(reply) == XPC_TYPE_DICTIONARY) {
        const char *word = xpc_dictionary_get_string(reply, "reply");
        printf("host: reply word=%s\n", word ? word : "(none)");
        if (strcmp(mode, "seccode") == 0) {
            check_reply_code(reply);
        }
    }
    // 2 つ目の伝言（確かめに失敗した口に、続けて送ったものが届くか）
    xpc_object_t again = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(again, "op", "second");
    xpc_object_t reply2 = xpc_connection_send_message_with_reply_sync(conn, again);
    printf("host: mode=%s second reply %s\n", mode, describe(reply2));
    return 0;
}
