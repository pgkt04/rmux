/* Ported from tmux tty-term.c and tty-features.c @ 8f25579c. */
#define main draw_reference_main
#include "draw_reference.c"
#undef main
#include "tty-features.c"
struct options_entry *options_get_only(struct options *o,const char *name) {(void)o;(void)name;return NULL;}
struct options_array_item *options_array_first(struct options_entry *o) {(void)o;return NULL;}
struct options_array_item *options_array_next(struct options_array_item *o) {(void)o;return NULL;}
union options_value *options_array_item_value(struct options_array_item *o) {(void)o;return NULL;}
int main(void) {
 const char *features[]={"RGB,usstyle,cstyle,utf8,hyperlinks","RGB@,RGB,mouse","clipboard","256,margins,rectfill","cstyle"};
 const char *overrides[]={"","","Ms=","setrgbf@:Clmg@:Rect@:am@","Ss=new:colors=bad:bel=\\E[1::2m"};
 u_int i,j;struct tty_term t;struct tty tty;struct client c;
 for(i=0;i<nitems(features);i++) {
  memset(&t,0,sizeof t);memset(&tty,0,sizeof tty);memset(&c,0,sizeof c);t.name="test";t.tty=&tty;tty.client=&c;c.name="test";
  t.codes=calloc(tty_term_ncodes(),sizeof *t.codes);
  setcap(&t,TTYC_CLEAR,strdup("x"));setcap(&t,TTYC_CUP,strdup("%i%p1%d;%p2%d"));setcap(&t,TTYC_SS,strdup("original"));
  t.codes[TTYC_AM].type=TTYCODE_FLAG;t.codes[TTYC_AM].value.flag=1;
  t.codes[TTYC_COLORS].type=TTYCODE_NUMBER;t.codes[TTYC_COLORS].value.number=8;
  tty_parse_client_features(&c,features[i],",");tty_apply_features(&t);
  tty_term_apply(&t,overrides[i],1,0);tty_term_apply_overrides(&t);
  printf("case %u %d %d\n",i,t.flags,!!(c.flags&CLIENT_UTF8));
  for(j=0;j<tty_term_ncodes();j++) {
   struct tty_code *code=&t.codes[j];const unsigned char *s;
   if(code->type==TTYCODE_NONE)continue;
   printf("%s=",tty_term_codes[j].name);
   if(code->type==TTYCODE_STRING)for(s=(unsigned char *)code->value.string;*s;s++)printf("%02x",*s);
   else if(code->type==TTYCODE_NUMBER)printf("%d",code->value.number);
   else printf("%d",code->value.flag);
   puts("");
  }
 }
 return 0;
}
