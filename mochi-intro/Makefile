CFLAGS = -Wall -g -O `pkg-config --cflags margo`
LDLIBS = `pkg-config --libs margo`

PROGS = server client
 
all: $(PROGS)

server.o: server.c types.h

client.o: client.c types.h

clean:
		rm -f $(PROGS) *.o