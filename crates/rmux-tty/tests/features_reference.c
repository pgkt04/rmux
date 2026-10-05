/* Ported from tmux tty-features.c @ 8f25579c. Test-only reference. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "tmux.h"
#include "tty-features.c"
void log_debug(const char *fmt, ...) {(void)fmt;}
__dead void fatalx(const char *fmt, ...) {(void)fmt;abort();}
__dead void fatal(const char *fmt, ...) {(void)fmt;abort();}
int main(void) {
 const char *names[]={"mintty","tmux","rxvt-unicode","iTerm2","foot","WezTerm","ghostty","Rio","XTerm","unknown"};
 struct client c; size_t i,j;
 for(i=0;i<nitems(names);i++) {
  memset(&c,0,sizeof c);tty_default_features(&c,names[i],1);
  printf("default %s %d %d\n",names[i],c.term_features,c.term_nofeatures);
 }
 for(i=0;i<nitems(tty_features);i++) {
  const struct tty_feature *f=tty_features[i];
  printf("feature %s %d\n",f->name,f->flags);
  if(f->capabilities)for(j=0;f->capabilities[j];j++)printf("cap %s\n",f->capabilities[j]);
 }
 memset(&c,0,sizeof c);tty_parse_client_features(&c,"rGb:RGB@:RGB:utf8:unknown:mouse",":");
 printf("parse %d %d\n",c.term_features,c.term_nofeatures);
 return 0;
}
