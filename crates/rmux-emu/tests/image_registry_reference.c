/* Ported from tmux image.c and image-sixel.c @ 8f25579c */
#define main codec_fixture_main
#include "sixel_reference.c"
#undef main
#include <sys/queue.h>
#ifndef TAILQ_FOREACH_SAFE
#define TAILQ_FOREACH_SAFE(v,h,f,t) for((v)=TAILQ_FIRST(h);(v)&&((t)=TAILQ_NEXT(v,f),1);(v)=(t))
#endif
#define printflike(a,b)
static int log_get_level(void) { return 0; }
static char *xstrdup(const char *s) { size_t n=strlen(s)+1; char *p=xmalloc(n); memcpy(p,s,n); return p; }
static int xasprintf(char **p,const char *fmt,...) { va_list ap; va_start(ap,fmt); int n=vsnprintf(NULL,0,fmt,ap); va_end(ap); *p=xmalloc(n+1); va_start(ap,fmt); vsnprintf(*p,n+1,fmt,ap); va_end(ap); return n; }
TAILQ_HEAD(images,image);
struct screen { unsigned cx,cy; struct images images; };
struct image { struct screen *s; struct sixel_image *data; unsigned px,py,sx,sy; char *fallback; struct images *list; TAILQ_ENTRY(image) entry; TAILQ_ENTRY(image) all_entry; };
#include "registry.c"
static struct screen screens[3];
static unsigned identity(struct image *im) { return im->data->p2; }
static void snapshot(void) {
    struct image *im;
    printf("global"); TAILQ_FOREACH(im,&all_images,all_entry) printf(" %u",identity(im)); puts("");
    for(unsigned s=0;s<3;s++) {
        printf("local%u",s); TAILQ_FOREACH(im,&screens[s].images,entry) printf(" %u",identity(im)); puts("");
        TAILQ_FOREACH(im,&screens[s].images,entry) {
            printf("image %u %u %u %u %u %u %u %u %u %u\n",identity(im),im->px,im->py,im->sx,im->sy,im->data->x,im->data->y,im->data->ra_x,im->data->ra_y,im->data->used_colours);
            for(size_t i=0;i<strlen(im->fallback);i++) printf("%02x",(unsigned char)im->fallback[i]); puts("");
        }
    }
}
int main(void) {
    for(unsigned i=0;i<3;i++) TAILQ_INIT(&screens[i].images);
    char op[20]; unsigned s,a,b,c,d,id;
    while(scanf("%19s",op)==1) {
        int result=0;
        if(!strcmp(op,"store")) { scanf("%u%u%u%u%u%u",&s,&id,&a,&b,&c,&d); char buf[80]; snprintf(buf,sizeof buf,"q#0;2;1;2;3\"1;1;%u;%u@",c,d); screens[s].cx=a; screens[s].cy=b; image_store(&screens[s],sixel_parse(buf,strlen(buf),id,2,3)); }
        else if(!strcmp(op,"remove")) { scanf("%u%u",&s,&id); struct image *im,*next; TAILQ_FOREACH_SAFE(im,&screens[s].images,entry,next) if(identity(im)==id) { image_free(im); result=1; break; } }
        else if(!strcmp(op,"free")) { scanf("%u",&s); result=image_free_all(&screens[s]); }
        else if(!strcmp(op,"line")) { scanf("%u%u%u",&s,&a,&b); result=image_check_line(&screens[s],a,b); }
        else if(!strcmp(op,"area")) { scanf("%u%u%u%u%u",&s,&a,&b,&c,&d); result=image_check_area(&screens[s],a,b,c,d); }
        else if(!strcmp(op,"scroll")) { scanf("%u%u",&s,&a); result=image_scroll_up(&screens[s],a); }
        else return 2;
        printf("result %d\n",result); snapshot();
    }
    for(unsigned i=0;i<3;i++) image_free_all(&screens[i]);
    return 0;
}
