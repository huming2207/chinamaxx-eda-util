<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE eagle SYSTEM "eagle.dtd">
<eagle version="9.6.2"><drawing><schematic>
<libraries><library name="Test"><devicesets>
<deviceset name="R"><devices><device name="" package="R_0603"><technologies><technology name=""><attribute name="LCSC" value="C25804"/></technology></technologies></device></devices></deviceset>
<deviceset name="C"><devices><device name="" package="C_0603"/></devices></deviceset>
<deviceset name="GND"><devices><device name=""/></devices></deviceset>
</devicesets></library></libraries>
<parts>
<part name="R1" library="Test" deviceset="R" device="" value="10k"/>
<part name="R2" library="Test" deviceset="R" device="" value="10k"/>
<part name="C1" library="Test" deviceset="C" device="" value="100n" populate="no"/>
<part name="GND1" library="Test" deviceset="GND" device=""/>
</parts>
<sheets><sheet><instances><instance part="R1" gate="G$1" x="100" y="200"/><instance part="R2" gate="G$1" x="100" y="300"/></instances></sheet></sheets>
</schematic></drawing></eagle>
