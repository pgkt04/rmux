/* Ported from tmux tty.c and tty-draw.c @ 8f25579c.
 * Test-only recorder: the pinned implementations write to a real evbuffer.
 */
#define HAVE_FLOCK 1
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"
#include "tty-term.c"
#undef lines
#include "tty.c"
#include "tty-draw.c"
#include "tty-acs.c"
#include "grid.c"
#include "grid-view.c"
#include "colour.c"
#include "hyperlinks.c"
#include "screen.c"
#include "screen-write.c"
struct options *global_options;
struct timeval start_time;
time_t current_time;
void log_debug(const char *fmt, ...) {(void)fmt;}
int log_get_level(void) {return 0;}
__dead void fatalx(const char *fmt, ...) {(void)fmt;abort();}
__dead void fatal(const char *fmt, ...) {(void)fmt;abort();}
const char *options_get_string(struct options *o,const char *n) {(void)o;if(!strcmp(n,"default-terminal"))return "tmux-256color";abort();}
int core_clear_on_attach;
long long options_get_number(struct options *o,const char *n) {(void)o;if(!strcmp(n,"clear-on-attach"))return core_clear_on_attach;if(!strcmp(n,"variation-selector-always-wide")||!strcmp(n,"extended-keys")||!strcmp(n,"focus-events"))return 0;abort();}
static u_int redraw_y, redraw_n;
static void output_ready(int fd,short events,void *arg) {(void)fd;(void)events;(void)arg;}
static void redraw(const struct tty_ctx *ctx,u_int y,u_int n) {(void)ctx;redraw_y=y;redraw_n=n;}
static void setcap(struct tty_term *t,enum tty_code_code code,const char *s) {t->codes[code].type=TTYCODE_STRING;t->codes[code].value.string=(char *)s;}
static void fixture(struct tty_term *t) {
 memset(t,0,sizeof *t);t->name="fixture";t->codes=calloc(tty_term_ncodes(),sizeof *t->codes);
 #define S(c,s) setcap(t,TTYC_##c,s)
 S(CLEAR,"\033[H\033[2J");S(CUP,"\033[%i%p1%d;%p2%dH");S(CSR,"\033[%i%p1%d;%p2%dr");
 S(SGR0,"\033[0m");S(CIVIS,"\033[?25l");S(CNORM,"\033[?25h");
 S(ICH,"\033[%p1%d@");S(ICH1,"\033[@");S(DCH,"\033[%p1%dP");S(DCH1,"\033[P");
 S(IL,"\033[%p1%dL");S(IL1,"\033[L");S(DL,"\033[%p1%dM");S(DL1,"\033[M");
 S(EL,"\033[K");S(EL1,"\033[1K");S(ECH,"\033[%p1%dX");S(ED,"\033[J");
 S(INDN,"\033[%p1%dS");S(RI,"\033M");S(RIN,"\033[%p1%dT");
 S(SETAB,"\033[4%p1%dm");S(SETAF,"\033[3%p1%dm");S(BOLD,"\033[1m");S(REV,"\033[7m");
 S(SMACS,"\016");S(RMACS,"\017");S(CMG,"\033[%i%p1%d;%p2%ds");S(CLMG,"\033[s");
 S(SYNC,"\033[?2026%?%p1%{1}%=%th%el%;");S(MS,"\033]52;%p1%s;%p2%s\007");
 #undef S
 t->codes[TTYC_AM].type=TTYCODE_FLAG;t->codes[TTYC_AM].value.flag=1;
 t->codes[TTYC_BCE].type=TTYCODE_FLAG;t->codes[TTYC_BCE].value.flag=1;
 t->codes[TTYC_AX].type=TTYCODE_FLAG;t->codes[TTYC_AX].value.flag=1;
 t->codes[TTYC_COLORS].type=TTYCODE_NUMBER;t->codes[TTYC_COLORS].value.number=8;
}
static char *tok(void){char *p=strtok(NULL," \n");if(!p)abort();return p;}
static u_int num(void){return strtoul(tok(),NULL,10);}
static int snum(void){return strtol(tok(),NULL,10);}
static size_t bytes(char *out){char *s=tok();size_t n=0;unsigned v;if(!strcmp(s,"-"))return 0;while(*s){if(sscanf(s,"%2x",&v)!=1)abort();out[n++]=v;s+=2;}out[n]=0;return n;}
static struct grid_cell gcparse(void){struct grid_cell gc=grid_default_cell;char b[64];gc.attr=num();gc.flags=num();gc.fg=snum();gc.bg=snum();gc.us=snum();gc.link=num();gc.data.width=num();gc.data.size=bytes(b);if(gc.data.size>UTF8_SIZE)abort();memcpy(gc.data.data,b,gc.data.size);return gc;}
int main(void) {
 struct tty t;struct tty_term term;struct client client;struct screen s;struct tty_ctx c;struct grid_cell gc;
 char line[100000],b[40000],*op;u_int w=0,h=0;int live=0;
 memset(&t,0,sizeof t);memset(&client,0,sizeof client);client.name="reference";client.flags=CLIENT_UTF8;client.theme=THEME_UNKNOWN;
 event_init();t.client=&client;t.term=&term;t.out=evbuffer_new();t.fg=t.bg=-1;fixture(&term);event_set(&t.event_out,-1,EV_WRITE,output_ready,&t);
 while(fgets(line,sizeof line,stdin)) {
  op=strtok(line," \n");if(!op)continue;
  if(!strcmp(op,"new")) {w=num();h=num();free(term.codes);fixture(&term);if(live)screen_free(&s);memset(&s,0,sizeof s);screen_init(&s,w,h,0);live=1;t.sx=w;t.sy=h;t.cx=t.cy=t.rupper=t.rlower=t.rleft=t.rright=UINT_MAX;t.cell=t.last_cell=grid_default_cell;t.mode=MODE_CURSOR;t.flags=0;evbuffer_drain(t.out,EVBUFFER_LENGTH(t.out));}
  else if(!strcmp(op,"capoff")){enum tty_code_code i=num();term.codes[i].type=TTYCODE_NONE;}
  else if(!strcmp(op,"termflags")){term.flags=num();}
  else if(!strcmp(op,"ttyflags")){t.flags=num();}
  else if(!strcmp(op,"cursor")){t.cx=num();t.cy=num();}
  else if(!strcmp(op,"lineflag")){u_int y=num();grid_get_line(s.grid,s.grid->hsize+y)->flags=num();}
  else if(!strcmp(op,"grid")){u_int x=num(),y=num();gc=gcparse();grid_view_set_cell(s.grid,x,y,&gc);}
  else if(!strcmp(op,"selection")){struct grid_cell selected=gcparse();screen_set_selection(&s,0,0,w-1,h-1,0,0,0,&selected);}
  else if(!strcmp(op,"line")){u_int x=num(),y=num(),n=num(),ax=num(),ay=num();tty_draw_line(&t,&s,x,y,n,ax,ay,NULL);}
  else if(!strcmp(op,"cmd")) {
   char *name=tok();memset(&c,0,sizeof c);c.s=&s;c.cell=&gc;c.defaults=grid_default_cell;c.style_ctx.defaults=&c.defaults;c.redraw_cb=redraw;
   c.ocx=num();c.ocy=num();c.orupper=num();c.orlower=num();c.xoff=snum();c.yoff=snum();c.rxoff=snum();c.ryoff=snum();c.sx=num();c.sy=num();c.wox=num();c.woy=num();c.wsx=num();c.wsy=num();c.bg=num();c.flags=num();c.n=num();gc=grid_default_cell;
   if(!strcmp(name,"cell")||!strcmp(name,"cells"))gc=gcparse();
   if(!strcmp(name,"cells")||!strcmp(name,"rawstring")){c.data.size=bytes(b);c.data.data=b;}
   if(!strcmp(name,"setselection")){c.sel.clip="c";c.sel.size=bytes(b);c.sel.data=b;}
   #define CMD(n) if(!strcmp(name,#n))tty_cmd_##n(&t,&c);else
   CMD(insertcharacter) CMD(deletecharacter) CMD(clearcharacter) CMD(insertline) CMD(deleteline)
   CMD(clearline) CMD(clearendofline) CMD(clearstartofline) CMD(reverseindex) CMD(linefeed)
   CMD(scrollup) CMD(scrolldown) CMD(clearendofscreen) CMD(clearstartofscreen) CMD(clearscreen)
   CMD(alignmenttest) CMD(cell) CMD(cells) CMD(redrawline) CMD(setselection) CMD(rawstring) CMD(syncstart) abort();
   #undef CMD
  } else if(!strcmp(op,"syncend"))tty_sync_end(&t);
  else if(!strcmp(op,"dump")) {
   size_t n=EVBUFFER_LENGTH(t.out),i;unsigned char *p=evbuffer_pullup(t.out,-1);
   printf("%u %u %u %u %u %u %u %u %u ",t.cx,t.cy,t.rupper,t.rlower,t.rleft,t.rright,t.flags,redraw_y,redraw_n);
   for(i=0;i<n;i++)printf("%02x",p[i]);puts("");evbuffer_drain(t.out,n);redraw_y=redraw_n=0;
  } else abort();
 }
 if(live)screen_free(&s);event_del(&t.event_out);free(term.codes);evbuffer_free(t.out);return 0;
}
