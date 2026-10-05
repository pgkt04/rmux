/* Ported from tmux image-sixel.c @ 8f25579c. Standalone fixture support. */
#include <stdio.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include <errno.h>
typedef unsigned int u_int;
#define SIXEL_COLOUR_REGISTERS 1024
static void log_debug(const char *fmt, ...) { (void)fmt; }
static void *xmalloc(size_t n) { void *p = malloc(n); if (!p) abort(); return p; }
static void *xcalloc(size_t n, size_t s) { void *p = calloc(n, s); if (!p) abort(); return p; }
static void *xreallocarray(void *p, size_t n, size_t s) { p = realloc(p, n*s); if (!p) abort(); return p; }
static void *xrecallocarray(void *p, size_t old, size_t n, size_t s) { p = xreallocarray(p,n,s); memset((char *)p + old*s,0,(n-old)*s); return p; }
static int xsnprintf(char *p, size_t n, const char *fmt, ...) { va_list ap; va_start(ap,fmt); int r = vsnprintf(p,n,fmt,ap); va_end(ap); if (r < 0 || (size_t)r >= n) abort(); return r; }
static long long strtonum(const char *s, long long min, long long max, const char **error) { char *end; errno=0; long long n=strtoll(s,&end,10); *error = errno || *end || n<min || n>max ? "range" : NULL; return *error ? 0 : n; }
struct sixel_image;
void sixel_free(struct sixel_image *);
void sixel_size_in_cells(struct sixel_image *, u_int *, u_int *);
#include "codec.c"
static void dump(struct sixel_image *si, struct sixel_image *map) {
    if (!si) { puts("bad"); return; }
    unsigned cx,cy; sixel_size_in_cells(si,&cx,&cy);
    printf("%u %u %u %u %u %u %u %u %u %u %u\n",si->x,si->y,si->xpixel,si->ypixel,si->set_ra,si->ra_x,si->ra_y,si->used_colours,si->p2,cx,cy);
    for(unsigned i=0;i<si->ncolours;i++) printf("%u,",si->colours[i]); puts("");
    uint64_t hash=14695981039346656037ULL;
    for(unsigned y=0;y<si->y;y++) { hash=(hash^si->lines[y].x)*1099511628211ULL; for(unsigned x=0;x<si->lines[y].x;x++) hash=(hash^si->lines[y].data[x])*1099511628211ULL; }
    printf("%llu\n",(unsigned long long)hash);
    size_t n=0; char *p=sixel_print(si,map,&n);
    if(!p) puts("none"); else { for(size_t i=0;i<n;i++) printf("%02x",(unsigned char)p[i]); puts(""); free(p); }
}
int main(int argc,char **argv) {
    if(argc!=13) return 2;
    size_t n; char *buf;
    if(argv[1][0]=='@') { FILE *f=fopen(argv[1]+1,"rb"); if(!f) return 3; fseek(f,0,SEEK_END); n=(size_t)ftell(f); rewind(f); buf=xcalloc(n+1,1); if(fread(buf,1,n,f)!=n) return 4; fclose(f); }
    else { n=strlen(argv[1])/2; buf=xcalloc(n+1,1); for(size_t i=0;i<n;i++) { unsigned ch; sscanf(argv[1]+2*i,"%2x",&ch); buf[i]=(char)ch; } }
    unsigned a[11]; for(int i=0;i<11;i++) a[i]=(unsigned)strtoul(argv[i+2],NULL,10);
    struct sixel_image *si=sixel_parse(buf,n,a[0],a[1],a[2]); free(buf);
    if(!si) { puts("bad"); return 0; }
    if(a[3]) { struct sixel_image *scaled=sixel_scale(si,a[4],a[5],a[6],a[7],a[8],a[9],a[10]); dump(scaled,a[10]?NULL:si); if(scaled) sixel_free(scaled); }
    else dump(si,NULL);
    sixel_free(si); return 0;
}
