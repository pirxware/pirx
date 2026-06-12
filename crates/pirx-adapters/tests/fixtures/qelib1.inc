gate id a { U(0,0,0) a; }
gate x a { U(pi,0,pi) a; }
gate y a { U(pi,pi/2,pi/2) a; }
gate z a { U(0,0,pi) a; }
gate h a { U(pi/2,0,pi) a; }
gate s a { U(0,0,pi/2) a; }
gate sdg a { U(0,0,-pi/2) a; }
gate t a { U(0,0,pi/4) a; }
gate tdg a { U(0,0,-pi/4) a; }
gate sx a { U(pi/2,-pi/2,pi/2) a; }
gate rx(theta) a { U(theta,-pi/2,pi/2) a; }
gate ry(theta) a { U(theta,0,0) a; }
gate rz(phi) a { U(0,0,phi) a; }
gate p(lambda) a { U(0,0,lambda) a; }
gate cx a,b { CX a,b; }
gate cz a,b { h b; cx a,b; h b; }
gate swap a,b { cx a,b; cx b,a; cx a,b; }
gate ccx a,b,c { h c; cx b,c; tdg c; cx a,c; t c; cx b,c; tdg c; cx a,c; t b; t c; h c; cx a,b; t a; tdg b; cx a,b; }
