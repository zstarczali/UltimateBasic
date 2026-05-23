<code>
	; ----------------------------------------------------------------------------------------------------
	;
	;	afli-plasma-effekt
	;	----------------------
	;
	;	coding: testicle/payday
	;	logo: fabu/payday
	;	musik: htd/topaz beerline
	;	1x1-char: testicle/payday
	;
	;
	;	contact and payday-releases:
	;	------------------------------------
	;
	;	daniel@popelganda.de
	;	www.popelganda.de
	;
	;
	;	this sourcecode is best view with the font "tahoma", font size 9.
	;	you can compile this code using the ACME crossassembler. get the complete
	;	package including resource files at www.popelganda.de
	;
	;	the code was written with Relaunch64, the c64-crossassembler-tool
	;	for windows-pc. grab it at www.popelganda.de!
	;
	; ----------------------------------------------------------------------------------------------------


; --------------------------------------------------
;----- Paragraph @globale variablen@ -----
; --------------------------------------------------

nextpart=$080b
sinus1 =$0a00
sinus2 =$0c00
sinus3 =$0e00
chartab=$9a00
aflicolor=$9c00
paydata=$2800
stext = $3800

!to "afli-plasma.prg"


;--------------------------------------------------
;----- Paragraph @includes of binaries@ -----
;--------------------------------------------------

*= $0800
!byte $00,$0c,$08,$0a, $00,$9e,$33,$32,$37,$36,$38,$00,$00,$00,$00

*= sinus1
!bin "d018-tab1.bin"

*= sinus2
!bin "d018-tab4.bin"

*= sinus3
!bin "d018-tab5.bin"

*= $1000
!bin "music.bin"

*= $2000
!bin "payday-char.bin"

*= $2800
!bin "payday-data.bin"

*= $3000
!bin "1x1char.bin"

*= aflicolor
!bin "afli-cols1.bin"

;*= $c800
;!bin "sonstiges/fastload/fastload.bin"



; --------------------------------------------------
;----- Paragraph @start of sourcecode@ -----
; --------------------------------------------------

*= $8000
		
		lda #0
		jsr $e536

		sei
		lda #<firq1
		sta $0314
		lda #>firq1
		sta $0315

		lda #1			;irq-set-up
		sta $d01a
		lda #$7f
		sta $dc0d
		lda #$a8
		sta $d012
		lda #$7b
		sta $d011

		lda #0
		sta $d020
		sta $d021
		sta $d001
		sta $dc0e
		sta $40
		sta $41

		jsr $1000		;sound-init

		lda #0			;init text-counter
		sta $50
		lda #>stext
		sta $51

		ldx #0
		lda #%01010001	;bitmap for fli-plasma
.loop		sta $6140,x
		sta $6180,x
		inx:bne .loop
		jsr .logoinit
		cli
loading	lda #0			;wait for signal-byte set... (during scroller)
		beq loading

		jsr $c800		;...then start loading next part
		ldx #8
		ldy #1
		jsr $ffba
		lda #2
		ldx #<.fname
		ldy #>.fname
		jsr $ffbd
		lda #0
		jsr $ffd5
		jmp nextpart

.fname
!ct pet
!tx "2*"


	; --------------------------------------------------
	;----- Paragraph @sub-route: logo init@ -----
	; --------------------------------------------------

.logoinit	ldx #0
.loop1		lda paydata+$80,x
		sta $04f0,x
		inx
		bne .loop1

		ldx #103
.loop2		lda paydata+$180,x
		sta $05f0,x
		dex
		bpl .loop2

		ldx #0
		lda #8
.loop3		sta $d8f0,x
		sta $d9f0,x
 		inx
		bne .loop3
		ldx #0
		lda #0
.loop4		sta $0400,x
		inx:cpx #$f0
		bne .loop4
		ldx #39
		lda #15
.loop5		sta $d878,x
		dex:bpl .loop5
		rts


;----------------- ende -------------------------
		


; --------------------------------------------------
;----- Paragraph @dummy-irq: logo fade-in@ -----
; --------------------------------------------------
		
!zone		
firq1		inc $d019
		lda #$d8
		sta $d016
		lda #$1b
		sta $d011
		lda #$19
		sta $d018
		
.col1		lda #0			;fade-in of payday-logo
		sta $d023
.col2		lda #0
		sta $d022
.col3		lda #0
		sta $d021
		
		jsr $1003		;sound
		
.wait		lda #1
		dec .wait+1
		lda .wait+1
		bne .weiter
		
.cnt		ldx #0			;increase counter for logo-fade-in
		lda pfcol,x
		sta .col1+1
		lda pfcol+1,x
		sta .col2+1
		lda pfcol+2,x
		sta .col3+1
		lda pfcol+3,x
		sta .wait+1
		lda .cnt+1
		clc:adc #4
		sta .cnt+1
		cmp #48
		bcc .weiter

		lda #$8c
		sta frast+1
		lda #<irq2
		sta fwait1+1
		lda #>irq2
		sta fwait2+1
		
.weiter	lda #<firq2
		sta $0314
		lda #>firq2
		sta $0315
		lda #$30
		sta $d012
		jmp $ea7e


;----- Paragraph @fade-in-farben für payday-logo@ -----

pfcol
!byte $00,$00,$00,$10
!byte $00,$00,$09,$02,$00,$09,$08,$02
!byte $09,$08,$0c,$02,$08,$0c,$0f,$02
!byte $0c,$0f,$01,$02,$0f,$01,$01,$02
!byte $01,$01,$0f,$02,$01,$0f,$0c,$02
!byte $0f,$0c,$08,$02,$0c,$08,$09,$48
!byte $0c,$08,$09,$02

;----------------- ende -------------------------



; --------------------------------------------------
;----- Paragraph @dummy-irq2: logo fade-in@ -----
;
;	this irq simulates a fli-routine, so the logo
;	is on the right position before the plasma
;	starts (switching on the plasma (fli) means that
;	the screenarea is moved down)
; --------------------------------------------------
		
!zone		
firq2		inc $d019
		lda #0
		sta $d021

.loop1		lda #$3a		;wait for rasterline
		cmp $d012
		bne .loop1

		ldy #10			;exact fli-timing
.loop2		dey
		bne .loop2
		lda #$7b
		sta $d011
		nop:nop
		cmp ($00,x)

		ldx #0
.loop3		lda #$7c		;fli-routine with turned off (black) screen
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$7d
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$7e
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$7f
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$78
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$79
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$7a
		sta $d011
		cmp ($00,x)
		cmp ($00,x)
		nop
		lda #$7b
		sta $d011
		bit $ea
		nop:nop
		inx:cpx #10
		bne .loop3
				
fwait1	lda #<firq1

		sta $0314
fwait2	lda #>firq1
		sta $0315
frast		lda #$a8
		sta $d012
		jmp $ea7e

;----------------- ende -------------------------



; --------------------------------------------------
;
;----- Paragraph @first irq: show afli-plasma@ -----
;
; --------------------------------------------------

!zone
irq1		inc $d019
fixit		lda #$c7
		sta $d016
		lda #2
		sta $dd00

		ldy #$3c

		lda #$7c
.loop1		cpy $d012
		bne .loop1

start
!set m=0
!do {
		sta $d011		;here we go. afli-routine
		lda chartab+m		;$d018-values
		sta $d018
		lda #$00
		sta $d016
		lda #$3d
		sta $d011
		lda chartab+1+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3e
		sta $d011
		lda chartab+2+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3f
		sta $d011
		lda chartab+3+m
		sta $d018
		lda #$00
		sta $d016
		lda #$38
		sta $d011
		lda chartab+4+m
		sta $d018
		lda #$00
		sta $d016
		lda #$39
		sta $d011
		lda chartab+5+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3a
		sta $d011
		lda chartab+6+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3b
		sta $d011
		lda chartab+7+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3c
		sta $d011
		lda chartab+8+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3d
		sta $d011
		lda chartab+9+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3e
		sta $d011
		lda chartab+10+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3f
		sta $d011
		lda chartab+11+m
		sta $d018
		lda #$00
		sta $d016
		lda #$38
		sta $d011
		lda chartab+12+m
		sta $d018
		lda #$00
		sta $d016
		lda #$39
		sta $d011
		lda chartab+13+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3a
		sta $d011
		lda chartab+14+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3b
		sta $d011
		lda chartab+15+m
		sta $d018
		lda #$00
		sta $d016
		lda #$3c
!set m=m+16
} until m = 80

		lda #$7b
		sta $d011		;turn screen off to avoid flickering

		lda #3
		sta $dd00
		lda #$19
		sta $d018
		lda #$8c
		sta $d012
		lda #<irq2
		sta $0314
		lda #>irq2
		sta $0315
		jmp $ea7e


;----------------- ende -------------------------



; --------------------------------------------------
;
;----- Paragraph @second irq: 1x1 scroller@ -----
;
; --------------------------------------------------

!zone
irq2		inc $d019
		lda #3
		sta $dd00
		lda #$1d
		sta $d018
		lda #$1b
		sta $d011

scroller	lda #$c7
		sta $d016

		jsr $1003		;sound

.schluss	jmp space
space		lda $d001		;space-key pressed?
		cmp #$ef
		beq .undweg
		jmp .ende
.undweg	lda #<.bildweg
		sta .schluss+1
		lda #>.bildweg
		sta .schluss+2
		jmp .ende

.bildweg	lda #1			;fade-out when space pressed
		dec .bildweg+1
		lda .bildweg+1
		bne .ende
		
.cnt		ldx #36			;pic-fadeout
		lda pfcol,x
		sta fout1+1
		lda pfcol+1,x
		sta fout1+1
		lda pfcol+2,x
		sta fout3+1
		lda pfcol+3,x
		sta .bildweg+1
		lda .cnt+1
		sec:sbc #4
		sta .cnt+1
		bcs .ende
		lda #0
		sta fout1+1
		sta fout2+1
		sta fout3+1
		lda #<.soundweg
		sta .schluss+1
		lda #>.soundweg
		sta .schluss+2
		jmp .ende

.soundweg	lda #15			;sound fadeout
		sta $d418
		dec .soundweg+1
		lda .soundweg+1
		cmp #$ff
		bne .ende
		
		lda #$7b
		sta $d011
		lda #0
		sta $d418
		sta $d015
		lda #$31
		sta $0314
		lda #$ea
		sta $0315
		lda #0
		sta $d01a
		sta $d020
		sta $d021
		jsr $e536
		lda #$81
		sta $dc0d
		lda #1			;set loading-signal for loading next part
		sta loading+1
		jmp $ea7e

.ende		lda #<irq3
		sta $0314
		lda #>irq3
		sta $0315
		lda #$a8
		sta $d012
		jmp $ea7e

;----------------- ende -------------------------



; --------------------------------------------------
;
;----- Paragraph @third irq: display logo and afli-action-routine@ -----
;
; --------------------------------------------------

!zone
irq3		inc $d019
		lda #$19		;change charset to logo-char
		sta $d018
		lda #$d8		;multicolor on
		sta $d016
		lda #$1b
		sta $d011
		
fout1		lda #12			;colours of logo
		sta $d023
fout2		lda #8
		sta $d022
fout3		lda #9
		sta $d021

		jsr .scrollroute		;1x1 scroller

.cnt		ldx #0
		ldy #0
.loop		lda aflicolor,x		;set new afli-plasma-colours
		sta $4028,y
		lda aflicolor+2,x
		sta $4428,y
		lda aflicolor+4,x
		sta $4828,y
		lda aflicolor+6,x
		sta $4c28,y
		lda aflicolor+8,x
		sta $5028,y
		lda aflicolor+10,x
		sta $5428,y
		lda aflicolor+12,x
		sta $5828,y
		lda aflicolor+14,x
		sta $5c28,y
		lda aflicolor+16,x
		sta $6028,y
		lda aflicolor+18,x
		sta $6428,y
		lda aflicolor+20,x
		sta $6828,y
		lda aflicolor+22,x
		sta $6c28,y
		lda aflicolor+24,x
		sta $7028,y
		lda aflicolor+26,x
		sta $7428,y
		lda aflicolor+28,x
		sta $7828,y
		lda aflicolor+30,x
		sta $7c28,y
		inx:iny
		cpy #40
		bne .loop

sinchange	jsr sinroute1

		inc $40			;change sinus-counter
		dec $41
		dec $41
		inc .pause+1
.pause	lda #0			;pause for sinus-counter
		and #1
		bne rwait1
		inc .cnt+1
		lda .cnt+1		;change sinus-counter for changing the afli-colours
		cmp #$6c
		bne rwait1
		lda #0
		sta .cnt+1
rwait1	lda #<firq2
		sta $0314
rwait2	lda #>firq2
		sta $0315
		lda #$38
		sta $d012
		lda #$1b
		sta $d011
		lda #0
		sta $d021
		jmp $ea7e

;----------------- ende -------------------------

	; --------------------------------------------------
	;----- Paragraph @sub-route: 1x1-scroller@ -----
	; --------------------------------------------------

.scrollroute	dec scroller+1		;soft-movement 1x1-scroller
		lda scroller+1		
		cmp #$bf
		bne .sweiter
		lda #$c7		;if 7 pixel moved, the scrolltext
		sta scroller+1		;hardscroll by one char

		ldx #0
.sloop		lda $0479,x
		sta $0478,x
		inx:cpx #40
		bne .sloop

		ldy #0
		lda ($50),y		;read new char
		bne .setchar		;endsign?
		lda #0			;if yes, reset text-counter
		sta $50
		lda #>stext
		sta $51
		lda #$20
.setchar	cmp #255		;startsign for afli-plasma?
		bne .setchar2
		lda #<irq1		;if yes, update irq-vector
		sta rwait1+1
		lda #>irq1
		sta rwait2+1
		lda #$20
.setchar2	cmp #254		;new sinus?

		bne .setchar3
		lda #<sinroute2
		sta sinchange+1
		lda #>sinroute2
		sta sinchange+2
		lda #$20
.setchar3	cmp #253		;new sinus?
		bne .setchar4
		lda #<sinroute3
		sta sinchange+1
		lda #>sinroute3
		sta sinchange+2
		lda #$20
.setchar4	cmp #252		;new sinus?
		bne .setchar5
		lda #<sinroute1
		sta sinchange+1
		lda #>sinroute1
		sta sinchange+2
		lda #$20
.setchar5	cmp #251
		bne .setchar6
		lda #$dc
		sta space+2
		lda #$20
.setchar6	sta $049f
		inc $50
		lda $50
		bne .sweiter
		inc $51
.sweiter	rts

;----------------- ende -------------------------

	; --------------------------------------------------
	;----- Paragraph @sub-route: sinus1@ -----
	; --------------------------------------------------

!zone
sinroute1	ldx $40
		ldy $41
.loop2		lda sinus1,x		;calculate new sinus (movement)
		clc:adc sinus1,y
		bcc .weit1
		eor #$f8
		ora #8
		jmp .weit2
.weit1		and #$f8
		ora #8
.weit2		sta chartab		;and update chartable ($d018-werte) for
		inx:iny			;afli-routine (irq1)
		inc .weit2+1
		lda .weit2+1
		cmp #80
		bne .loop2
		lda #0
		sta .weit2+1

		ldx $40
		ldy $41
!set p=0
!do {
		lda sinus1+p,x		;read $d016-values from sinus...
		clc:adc sinus1+p,y
		and #7:eor #$c7
;		and #7:eor #$c0
		sta start+10+p*$10	;...ans store them in afli-routine (irq1)
!set p=p+1
} until p = 80
		
		rts		

;----------------- ende -------------------------


	; --------------------------------------------------
	;----- Paragraph @sub-route: sinus2@ -----
	; --------------------------------------------------

!zone
sinroute2	ldx $40
		ldy $41
.loop2		lda sinus2,x		;sinus neu berechnen (movement)
		clc:adc sinus2,y
		bcc .weit1
		eor #$f8
		ora #8
		jmp .weit2
.weit1		and #$f8
		ora #8
.weit2		sta chartab		;und die chartabelle ($d018-werte) für
		inx:iny			;afli-routine (irq1) neu setzen
		inc .weit2+1
		lda .weit2+1
		cmp #80
		bne .loop2
		lda #0
		sta .weit2+1

		ldx $40
		ldy $41
!set p=0
!do {
		lda sinus2+p,x		;die $d016-werte aus dem sinus auslesen...
		clc:adc sinus2+p,y
		and #7:eor #$c7
;		and #7:eor #$c0
		sta start+10+p*$10	;...und in die afli-routine (irq1) setzen
!set p=p+1
} until p = 80
		rts		

;----------------- ende -------------------------


	; --------------------------------------------------
	;----- Paragraph @sub-route: sinus3@ -----
	; --------------------------------------------------

!zone
sinroute3	ldx $40
		ldy $41
.loop2		lda sinus3,x		;sinus neu berechnen (movement)
		clc:adc sinus3,y
		bcc .weit1
		eor #$f8
		ora #8
		jmp .weit2
.weit1		and #$f8
		ora #8
.weit2		sta chartab		;und die chartabelle ($d018-werte) für
		inx:iny			;afli-routine (irq1) neu setzen
		inc .weit2+1
		lda .weit2+1
		cmp #80
		bne .loop2
		lda #0
		sta .weit2+1

		ldx $40
		ldy $41
!set p=0
!do {
		lda sinus3+p,x		;die $d016-werte aus dem sinus auslesen...
		clc:adc sinus3+p,y
		and #7:eor #$c7
;		and #7:eor #$c0
		sta start+10+p*$10	;...und in die afli-routine (irq1) setzen
!set p=p+1
} until p = 80
		rts		

;----------------- ende -------------------------



; --------------------------------------------------
;----- Paragraph @scrolltext@ -----
; --------------------------------------------------

*= stext

!ct scr
!tx "          ha... the contribution for the forum-c64-competition from testicle/payday...     "
!tx "hm, where is it?       "

;sinus-start
!byte 255
!tx "ah yes! the credits: this small part was coded by me, testicle/payday. "

;sinus-wechsel
!byte 254
!tx "the payday-logo was painted years ago by fabu/payday. as i said, it's very old, but i unfortunately "
!tx "didn't have many graphics i could use."

;sinus-wechsel
!byte 253
!tx "the music was done by htd/topaz beerline. "
!tx "and last but not least this incredible charset, which was also done by me."


;sinus-wechsel
!byte 252
!tx "  by the way, if you like, press space to leave now."
!byte 251
!tx "the last time payday was active was in the year 1995."
!tx " we have - more or less periodical - released our discmag popelganda."

;sinus-wechsel
!byte 254
!tx "those who are interested in this mag can download all released issues at www.popelganda.de."


;sinus-wechsel
!byte 253
!tx "   now, in the year 2003, we want to contribute "
!tx "something to the c64-scene again and are currently working on the next issue of popelganda. everything's new, "
!tx "everything's different."

;sinus-wechsel
!byte 252
!tx "and popelganda will be kicking the scene, hehe...  ok, enough written.   text restarts!"
!tx "                                      "

;endzeichen
!byte 0
</code>