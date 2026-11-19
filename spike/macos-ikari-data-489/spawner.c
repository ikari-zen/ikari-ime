// (d) 束の中の別 exe（設定アプリの代わり。Sandbox なし）から posix_spawn で起こす。argv[1]=起こす exe
#include <spawn.h>
#include <stdio.h>
#include <sys/wait.h>
extern char **environ;
int main(int argc, char **argv) {
  pid_t pid; int rc = posix_spawn(&pid, argv[1], NULL, NULL, argv + 1, environ);
  printf("spawn rc=%d\n", rc); fflush(stdout);
  if (rc) return 1;
  int st; waitpid(pid, &st, 0);
  printf("child exit=%d signal=%d\n", WIFEXITED(st) ? WEXITSTATUS(st) : -1, WIFSIGNALED(st) ? WTERMSIG(st) : 0);
  return 0;
}
