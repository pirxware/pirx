OPENQASM 3.0;
include "stdgates.inc";

qubit[3] q;
bit[3] c;

h q[0];
cx q[0], q[1];
t q[0];
tdg q[1];
rz(pi/4) q[2];
rz(0.3) q[1];
s q[2];

c[0] = measure q[0];
c[1] = measure q[1];
c[2] = measure q[2];
