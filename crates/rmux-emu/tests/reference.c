/* Test driver for unmodified tmux sources at 8f25579c. */
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"
#include "colour.c"
#include "attributes.c"
#include "hyperlinks.c"
#include "style.c"
struct options *global_options;
const struct grid_cell grid_default_cell = {{{' '},0,1,1},0,0,8,8,8,0};
void log_debug(const char *fmt, ...) {(void)fmt;}
__dead void fatalx(const char *fmt, ...) {(void)fmt;abort();}
__dead void fatal(const char *fmt, ...) {(void)fmt;abort();}
struct options_entry *options_get(struct options *o,const char *n) {(void)o;(void)n;return NULL;}
struct options_array_item *options_array_first(struct options_entry *o) {(void)o;return NULL;}
struct options_array_item *options_array_next(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_item_value(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_getv(struct options_entry *o,const char *fmt,...) {(void)o;(void)fmt;return NULL;}
const struct options_table_entry *options_table_entry(struct options_entry *o) {(void)o;abort();}
const char *options_get_string(struct options *o,const char *n) {(void)o;(void)n;abort();}
static struct style *resolved_style;
struct style *options_string_to_style(struct options *o,const char *n,struct format_tree *f) {(void)o;(void)n;(void)f;return resolved_style;}
struct format_tree *format_create(struct client *c,struct cmdq_item *q,int a,int b) {(void)c;(void)q;(void)a;(void)b;abort();}
void format_free(struct format_tree *f) {(void)f;abort();}
char *format_single(struct cmdq_item *q,const char *s,struct client *c,struct session *se,struct winlink *w,struct window_pane *p) {(void)q;(void)s;(void)c;(void)se;(void)w;(void)p;abort();}
static void hex(const char *s) {if(s==NULL){printf("-");return;}for(;*s;s++)printf("%02x",(unsigned char)*s);}
static void unhex(char *s,char *out) {unsigned int n;while(*s && sscanf(s,"%2x",&n)==1){*out++=n;s+=2;}*out=0;}
static void state(struct style *s) {
 printf("%d %d %d %u %u %u %d %u %d %d %d %u ",s->gc.fg,s->gc.bg,s->gc.us,s->gc.attr,s->gc.flags,s->gc.link,s->ignore,s->dim,s->fill,s->align,s->range_type,s->range_argument);
 hex(s->range_string);printf(" %d %d %d %d %u ",s->width,s->width_percentage,s->pad,s->default_type,s->link);
 hex(style_tostring(s));printf(" %u ",global_hyperlinks_count);hex(style_link(s));puts("");
}
int main(void) {
 setlocale(LC_CTYPE,"en_US.UTF-8");if(MB_CUR_MAX==1)setlocale(LC_CTYPE,"C.UTF-8");utf8_update_width_cache();
 char line[40000],a[18000],*op,*arg,*arg2;struct style sy;struct grid_cell base=grid_default_cell;
 base.fg=1;base.bg=2;base.us=3;base.attr=GRID_ATTR_BRIGHT;base.flags=GRID_FLAG_SELECTED;base.link=77;base.data.data[0]='x';
 style_set(&sy,&base);struct hyperlinks *stores[16]={0};struct colour_palette palette;colour_palette_init(&palette);
 while(fgets(line,sizeof line,stdin)) {
  line[strcspn(line,"\n")]=0;op=strtok(line," ");arg=strtok(NULL," ");arg2=strtok(NULL," ");if(!op)continue;
  if(!strcmp(op,"C")||!strcmp(op,"N")||!strcmp(op,"X")||!strcmp(op,"A")||!strcmp(op,"S")||!strcmp(op,"B"))unhex(arg?arg:"",a);
  if(!strcmp(op,"C"))printf("%d\n",colour_fromstring(a));
  else if(!strcmp(op,"N"))printf("%d\n",colour_byname(a));
  else if(!strcmp(op,"X"))printf("%d\n",colour_parseX11(a));
  else if(!strcmp(op,"A"))printf("%d\n",attributes_fromstring(a));
  else if(!strcmp(op,"V")){hex(attributes_tostring(atoi(arg)));puts("");}
  else if(!strcmp(op,"R")){int c=atoi(arg),dim=atoi(arg2);printf("%d %d %d %d %d ",colour_256toRGB(c),colour_256to16(c),colour_force_rgb(c),colour_dim(c,dim),colour_totheme(c));hex(colour_tostring(c));puts("");}
  else if(!strcmp(op,"Q")){struct client c={0};struct tty_term term={0};c.tty.flags=TTY_OPENED;c.tty.term=&term;term.flags=atoi(arg2);hex(colour_toescape(&c,atoi(arg),0));putchar(' ');hex(colour_toescape(&c,atoi(arg),1));puts("");}
  else if(!strcmp(op,"F")){int r,g,b;sscanf(arg,"%d:%d:%d",&r,&g,&b);printf("%d\n",colour_find_rgb(r,g,b));}
  else if(!strcmp(op,"S")){printf("%d ",style_parse(&sy,&base,a));state(&sy);}
  else if(!strcmp(op,"B")){printf("%d ",style_parse_colour(&sy,&base,a));state(&sy);}
  else if(!strcmp(op,"I")){style_set(&sy,&base);state(&sy);}
  else if(!strcmp(op,"D")){sy=style_default;state(&sy);}
  else if(!strcmp(op,"overlay")){struct grid_cell c=base;resolved_style=&sy;style_add(&c,NULL,NULL,(struct format_tree *)1);printf("%d %d %d %u %u %u %u\n",c.fg,c.bg,c.us,c.attr,c.flags,c.link,c.data.data[0]);}
  else if(!strcmp(op,"fallback")){struct grid_cell c=base;resolved_style=NULL;style_add(&c,NULL,NULL,(struct format_tree *)1);printf("%d %d %d %u %u %u %u\n",c.fg,c.bg,c.us,c.attr,c.flags,c.link,c.data.data[0]);}
  else if(!strcmp(op,"pset")){printf("%d\n",colour_palette_set(&palette,atoi(arg),atoi(arg2)));}
  else if(!strcmp(op,"pget")){printf("%d\n",colour_palette_get(&palette,atoi(arg)));}
  else if(!strcmp(op,"pdefault")){if(palette.default_palette==NULL){palette.default_palette=xcalloc(256,sizeof(int));for(int n=0;n<256;n++)palette.default_palette[n]=-1;}palette.default_palette[atoi(arg)]=atoi(arg2);puts("ok");}
  else if(!strcmp(op,"pclear")){colour_palette_clear(&palette);printf("%d %d %d %d\n",palette.fg,palette.bg,palette.palette!=NULL,palette.default_palette!=NULL);}
  else if(!strcmp(op,"pfree")){colour_palette_free(&palette);printf("%d %d %d %d\n",palette.fg,palette.bg,palette.palette!=NULL,palette.default_palette!=NULL);}
  else if(!strcmp(op,"theme")){hex(colour_theme_option(atoi(arg),atoi(arg2)));printf(" %d\n",colour_theme_terminal_colour(atoi(arg)));}
  else if(!strcmp(op,"create")){int n=atoi(arg);stores[n]=hyperlinks_init();puts("ok");}
  else if(!strcmp(op,"share")){stores[atoi(arg2)]=hyperlinks_copy(stores[atoi(arg)]);puts("ok");}
  else if(!strcmp(op,"reset")){hyperlinks_reset(stores[atoi(arg)]);printf("%u\n",global_hyperlinks_count);}
  else if(!strcmp(op,"release")){hyperlinks_free(stores[atoi(arg)]);stores[atoi(arg)]=NULL;printf("%u\n",global_hyperlinks_count);}
  else if(!strcmp(op,"put")){char *id=strtok(NULL," ");char uri[18000],internal[18000];const char *result,*ext;unhex(arg2?arg2:"",uri);unhex(id?id:"",internal);unsigned int inner=hyperlinks_put(stores[atoi(arg)],uri,internal);printf("%u %u ",inner,global_hyperlinks_count);if(hyperlinks_get(stores[atoi(arg)],inner,&result,NULL,&ext))hex(ext);else printf("-");puts("");}
  else if(!strcmp(op,"get")){const char *uri,*id,*ext;if(hyperlinks_get(stores[atoi(arg)],atoi(arg2),&uri,&id,&ext)){hex(uri);putchar(' ');hex(id);putchar(' ');hex(ext);}else printf("-");puts("");}
  else if(!strcmp(op,"transfer")){const char *uri=style_link(&sy);printf("%u\n",uri?hyperlinks_put(stores[atoi(arg)],uri,uri):0);}
 }
 return 0;
}
