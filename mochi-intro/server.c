#include "types.h"
#include <assert.h>
#include <margo.h>
#include <stdio.h>

static void put(hg_handle_t h);
DECLARE_MARGO_RPC_HANDLER(put)

int main() {
    margo_instance_id mid;
    char addr_str[PATH_MAX];
    size_t addr_str_size = sizeof(addr_str);
    hg_addr_t my_address;

    mid = margo_init("sockets", MARGO_SERVER_MODE, 1, 1);
    assert(mid);

    margo_addr_self(mid, &my_address);
    margo_addr_to_string(mid, addr_str, &addr_str_size, my_address);
    margo_addr_free(mid, my_address);
    printf("Server running at address %s\n", addr_str);

    MARGO_REGISTER(mid, "put", put_in_t, int32_t, put);

    margo_wait_for_finalize(mid);

    return (0);
}

static void put(hg_handle_t h) {
    hg_return_t ret;
    put_in_t in;
    int32_t out;

    ret = margo_get_input(h, &in);
    assert(ret == HG_SUCCESS);

    printf("put: key %s, value %s\n", in.key, in.value);
    out = 0;

    ret = margo_free_input(h, &in);
    assert(ret == HG_SUCCESS);
    ret = margo_respond(h, &out);
    assert(ret == HG_SUCCESS);

    ret = margo_destroy(h);
    assert(ret == HG_SUCCESS);
}
DEFINE_MARGO_RPC_HANDLER(put)
