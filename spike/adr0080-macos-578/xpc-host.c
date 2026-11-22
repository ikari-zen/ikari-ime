// スパイク #578（ADR 0080 C1）。捨てるコード。マージしない。
// 束の中の殻の代わり（Sandbox なし）。XPC サービスに 1 回要求を送り、返事と往復の時間を出す。
// 使い方: xpc-host <サービス名> <deny に使うパス>
#include <xpc/xpc.h>
#include <stdio.h>
#include <time.h>

int main(int argc, char **argv) {
  if (argc < 3) { fprintf(stderr, "usage: xpc-host <service> <deny-path>\n"); return 2; }
  double t0 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1e6;
  xpc_connection_t c = xpc_connection_create(argv[1], NULL);
  xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
    if (xpc_get_type(ev) == XPC_TYPE_ERROR)
      fprintf(stderr, "connection event: %s\n", xpc_dictionary_get_string(ev, XPC_ERROR_KEY_DESCRIPTION));
  });
  xpc_connection_resume(c);
  xpc_object_t m = xpc_dictionary_create(NULL, NULL, 0);
  xpc_dictionary_set_string(m, "deny", argv[2]);
  xpc_object_t r = xpc_connection_send_message_with_reply_sync(c, m);
  double t1 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1e6;
  if (xpc_get_type(r) != XPC_TYPE_DICTIONARY) {
    printf("reply error: %s (%.1fms)\n", xpc_dictionary_get_string(r, XPC_ERROR_KEY_DESCRIPTION), t1 - t0);
    return 1;
  }
  printf("%s", xpc_dictionary_get_string(r, "out"));
  printf("xpc round trip (launch + probe): %.1fms\n", t1 - t0);
  return 0;
}
