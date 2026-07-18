.globl route_payment
route_payment:
  cmpq $0, %rdi
  jle .Lreject
  call submit_payment
  ret
.Lreject:
  call reject_payment
  ret
