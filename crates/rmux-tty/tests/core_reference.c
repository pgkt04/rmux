/* Ported from tmux tty.c @ 8f25579c. Test-only pinned implementation recorder. */
#define main draw_reference_main
#include "draw_reference.c"
#undef main
#include <fcntl.h>
#include <termios.h>
#if defined(__APPLE__)
#include <util.h>
#else
#include <pty.h>
#endif
void server_redraw_client(struct client *c) {(void)c;}
void setblocking(int fd,int block) {int f=fcntl(fd,F_GETFL);if(f!=-1)fcntl(fd,F_SETFL,block ? f&~O_NONBLOCK : f|O_NONBLOCK);}

static void core_fixture(struct tty_term *t) {
 fixture(t);
 #define S(c,s) setcap(t,TTYC_##c,s)
 S(CLEAR,"C");S(CUP,"<p%p1%d,%p2%d>");S(CSR,"<r%p1%d,%p2%d>");S(SGR0,"Z");
 S(SMCUP,"A");S(RMCUP,"a");S(SMKX,"K");S(RMKX,"k");S(CNORM,"N");S(CIVIS,"I");S(CVVIS,"V");
 S(HOME,"H");S(CUB1,"L");S(CUF1,"R");S(CUU1,"U");S(CUD1,"D");S(CUB,"<l%p1%d>");S(CUF,"<r%p1%d>");
 S(CUU,"<u%p1%d>");S(CUD,"<d%p1%d>");S(HPA,"<x%p1%d>");S(VPA,"<y%p1%d>");
 S(SETAF,"<f%p1%d>");S(SETAB,"<b%p1%d>");S(BOLD,"B");S(DIM,"d");S(SITM,"i");S(SMSO,"o");S(SMUL,"u");
 S(SMULX,"<u%p1%d>");S(BLINK,"b");S(REV,"v");S(INVIS,"h");S(SMXX,"x");S(SMOL,"t");S(SMACS,"s");S(RMACS,"e");
 S(SS,"<s%p1%d>");S(SE,"E");S(CR,"c");S(CS,"<c%p1%s>");S(MS,"<m%p1%s,%p2%s>");S(HLS,"<h%p1%s,%p2%s>");
 S(SETULC1,"<a%p1%d>");S(OL,"O");S(INDN,"<i%p1%d>");
 #undef S
 t->codes[TTYC_CMG].type=TTYCODE_NONE;t->codes[TTYC_CLMG].type=TTYCODE_NONE;
 t->codes[TTYC_SYNC].type=TTYCODE_NONE;
}
int main(void) {
 struct tty t;struct tty_term term;struct client client;struct grid_cell gc;
 char line[10000],buf[4096],*op;int master,slave;
 struct winsize ws={24,80,640,384};
 if(openpty(&master,&slave,NULL,NULL,&ws))abort();
 event_init();memset(&t,0,sizeof t);memset(&client,0,sizeof client);core_fixture(&term);
 client.name="reference";client.fd=slave;client.theme=THEME_UNKNOWN;t.client=&client;t.term=&term;t.out=evbuffer_new();
 tcgetattr(slave,&t.tio);t.sx=80;t.sy=24;t.fg=t.bg=t.ccolour=-1;t.cell=t.last_cell=grid_default_cell;
 t.rupper=0;t.rlower=23;t.rleft=0;t.rright=79;
 event_set(&t.event_in,slave,EV_READ,NULL,NULL);event_set(&t.event_out,slave,EV_WRITE,NULL,NULL);
 evtimer_set(&t.start_timer,NULL,&t);evtimer_set(&t.clipboard_timer,NULL,&t);evtimer_set(&t.timer,NULL,&t);
 while(fgets(line,sizeof line,stdin)) {
  op=strtok(line," \n");if(!op)continue;
  if(!strcmp(op,"cursor")){t.cx=num();t.cy=num();}
  else if(!strcmp(op,"move")){u_int x=num(),y=num();tty_cursor(&t,x,y);}
  else if(!strcmp(op,"region")){u_int u=num(),l=num();tty_region(&t,u,l);}
  else if(!strcmp(op,"attrs")){gc=gcparse();tty_attributes(&t,&gc,NULL);}
  else if(!strcmp(op,"reset"))tty_reset(&t);
  else if(!strcmp(op,"putc")){tty_putc(&t,num());}
  else if(!strcmp(op,"putn")){u_int w=num();size_t len=bytes(buf);tty_putn(&t,buf,len,w);}
  else if(!strcmp(op,"size")){t.sx=num();t.sy=num();}
  else if(!strcmp(op,"capoff")){enum tty_code_code i=num();term.codes[i].type=TTYCODE_NONE;}
  else if(!strcmp(op,"termflags")){term.flags=num();}
  else if(!strcmp(op,"ttyflags")){t.flags=num();}
  else if(!strcmp(op,"start")){core_clear_on_attach=num();tty_start_tty(&t);}
  else if(!strcmp(op,"stop")){
    /* Linux hands slave output to the master asynchronously: read until
     * 50 ms pass with no data, like the Rust side's read_settled. */
    tty_stop_tty(&t);fcntl(master,F_SETFL,O_NONBLOCK);
    for(int quiet=0,total=0;quiet<10&&total<400;total++){ssize_t n=read(master,buf,sizeof buf);if(n>0){evbuffer_add(t.out,buf,n);quiet=0;}else{quiet++;usleep(5000);}}
  }
  else if(!strcmp(op,"termios")){struct termios a;if(tcgetattr(slave,&a))abort();printf("termios %llu %llu %llu %llu %u %u\n",(unsigned long long)a.c_iflag,(unsigned long long)a.c_oflag,(unsigned long long)a.c_lflag,(unsigned long long)a.c_cflag,a.c_cc[VMIN],a.c_cc[VTIME]);}
  else if(!strcmp(op,"addcount")){u_int n=num();if(n>sizeof buf)abort();memset(buf,'x',n);tty_add(&t,buf,n);}
  else if(!strcmp(op,"block")){tty_block_maybe(&t);}
  else if(!strcmp(op,"blocktimer")){tty_timer_callback(0,0,&t);}
  else if(!strcmp(op,"redraw")){client.redraw=num();}
  else if(!strcmp(op,"writable")){tty_write_callback(0,0,&t);}
  else if(!strcmp(op,"clearout")){evbuffer_drain(t.out,EVBUFFER_LENGTH(t.out));}
  else if(!strcmp(op,"flowdump")) {
   size_t n=EVBUFFER_LENGTH(t.out),i;unsigned char *p=evbuffer_pullup(t.out,-1);
   printf("flow %u %zu %zu %zu %zu ",t.flags,n,client.discarded,t.discarded,client.redraw);
   for(i=0;i<n;i++)printf("%02x",p[i]);puts("");
  }
  else if(!strcmp(op,"dump")) {
   size_t n=EVBUFFER_LENGTH(t.out),i;unsigned char *p=evbuffer_pullup(t.out,-1);
   printf("%u %u %u %u %u %u %u %d %d %d %u ",t.cx,t.cy,t.rupper,t.rlower,t.rleft,t.rright,t.cell.attr,t.cell.fg,t.cell.bg,t.cell.us,t.cell.link);
   for(i=0;i<n;i++)printf("%02x",p[i]);puts("");evbuffer_drain(t.out,n);
  } else abort();
 }
 close(master);close(slave);return 0;
}
