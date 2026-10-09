OPENQASM 3.0;
include "stdgates.inc";
qubit q;
bit c;
c = measure q;
h q;
