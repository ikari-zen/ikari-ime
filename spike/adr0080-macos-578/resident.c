// スパイク #578。捨てるコード。ファイルを mmap して、mincore で常駐の割合だけを出す（ページは読み込まない）。
// 使い方: resident <file>...
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main(int argc, char **argv) {
  size_t ps = (size_t)getpagesize();
  for (int i = 1; i < argc; i++) {
    int fd = open(argv[i], O_RDONLY);
    struct stat st;
    if (fd < 0 || fstat(fd, &st) != 0) { perror(argv[i]); continue; }
    void *p = mmap(NULL, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    size_t pages = (st.st_size + ps - 1) / ps, n = 0;
    char *v = malloc(pages);
    if (p != MAP_FAILED && v && mincore(p, st.st_size, v) == 0)
      for (size_t k = 0; k < pages; k++) n += v[k] & 1;
    printf("%5.1f%%  %s\n", n * 100.0 / pages, argv[i]);
    free(v); munmap(p, st.st_size); close(fd);
  }
  return 0;
}
